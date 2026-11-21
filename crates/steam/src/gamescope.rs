//! Running a game inside Gamescope (Valve's micro-compositor, the one in the Steam Deck).
//!
//! Gamescope started from a desktop session opens one window and runs the game on its own private X server
//! (Xwayland), so the desktop can never minimize, un-fullscreen or cover the game, and an overlay window that
//! belongs to the same X server is drawn over it by Gamescope itself. This file builds the command and detects
//! the program; it starts nothing.
//!
//! # Command order
//! ```text
//! gamescope <options> -- sh -c <shim> kettle-gamescope <display file> <the command we would run anyway>
//! ```
//! "The command we would run anyway" is the whole plan: the user's launch-option wrappers (`gamemoderun`, a
//! `mangohud` wrapper ...), then the Steam Linux Runtime container / Proton / umu, then the game. Gamescope is the
//! outermost program, exactly what Steam Deck users get by writing `gamescope ... -- %command%`: it has to be
//! the parent of everything that opens a window, and the wrappers keep working on the game's own process tree.
//! Environment variables of the plan stay as they are (Gamescope and its child inherit them).
//!
//! MangoHud: with `mangoapp` installed the HUD is drawn by Gamescope (`--mangoapp`) and the game gets no
//! `MANGOHUD=1` and no `mangohud` wrapper (that would draw it twice); `MANGOHUD_CONFIGFILE` stays, mangoapp reads
//! it. Without `mangoapp` the usual MangoHud layer inside the game is kept. A frame rate cap goes to Gamescope
//! (`-r`); the HUD's own `fps_limit` is written as 0 then, so the game is not limited twice.
//!
//! # The display shim
//! Gamescope sets `DISPLAY` for its child to the private X server (`:N`). The overlay helper has to connect to
//! that server, so the child command is started through a three-line `sh` shim that writes `$DISPLAY` to a file
//! (`$XDG_RUNTIME_DIR/kettle-gamescope-<appid>.display`, atomically) and then `exec`s the real command. The main
//! app reads the file ([`read_display`]). It runs before any container, so it needs nothing inside it. Guessing
//! the display number instead would race with other X servers; the file cannot.

use crate::launch_opts::Parsed;
use crate::mangohud::{Hud, HudSettings};
use crate::run::LaunchPlan;
use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Largest resolution and frame rate accepted from a hand-edited file.
const MAX_DIMENSION: u32 = 16_384;
const MAX_FPS: u32 = 1000;
/// Gamescope's sharpness scale: 0 is the sharpest, 20 the softest.
pub const MAX_SHARPNESS: u8 = 20;

/// Commonly wanted sizes for the resolution lists (width, height). 0 x 0 = "like the screen".
pub const RESOLUTIONS: [(u32, u32); 7] =
    [(0, 0), (1280, 720), (1600, 900), (1920, 1080), (2560, 1440), (3440, 1440), (3840, 2160)];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    /// `-f`: Gamescope's window is fullscreen on the desktop.
    #[default]
    Fullscreen,
    /// `-b`: a window without borders.
    Borderless,
    /// A normal window.
    Windowed,
}

impl WindowMode {
    pub const ALL: [WindowMode; 3] = [WindowMode::Fullscreen, WindowMode::Borderless, WindowMode::Windowed];

    pub fn label(self) -> &'static str {
        match self {
            WindowMode::Fullscreen => "Fullscreen",
            WindowMode::Borderless => "Borderless window",
            WindowMode::Windowed => "Window",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Upscaler {
    /// No `-F`: Gamescope's own default.
    #[default]
    Default,
    Fsr,
    Nis,
    Linear,
    Nearest,
    Pixel,
}

impl Upscaler {
    pub const ALL: [Upscaler; 6] =
        [Upscaler::Default, Upscaler::Fsr, Upscaler::Nis, Upscaler::Linear, Upscaler::Nearest, Upscaler::Pixel];

    pub fn label(self) -> &'static str {
        match self {
            Upscaler::Default => "Gamescope default",
            Upscaler::Fsr => "AMD FSR",
            Upscaler::Nis => "NVIDIA NIS",
            Upscaler::Linear => "Linear",
            Upscaler::Nearest => "Nearest (sharp pixels)",
            Upscaler::Pixel => "Pixel (integer scaling)",
        }
    }

    /// The word after `-F`.
    pub fn flag(self) -> Option<&'static str> {
        match self {
            Upscaler::Default => None,
            Upscaler::Fsr => Some("fsr"),
            Upscaler::Nis => Some("nis"),
            Upscaler::Linear => Some("linear"),
            Upscaler::Nearest => Some("nearest"),
            Upscaler::Pixel => Some("pixel"),
        }
    }

    /// Does `--sharpness` do anything with this one?
    pub fn sharpens(self) -> bool {
        matches!(self, Upscaler::Fsr | Upscaler::Nis)
    }
}

/// The global setting "Run games in Gamescope" and its options. Saved in `settings.json` as `gamescope`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GamescopeSettings {
    /// The master switch (off by default). A game can override it, see [`Override`].
    pub enabled: bool,
    pub window: WindowMode,
    /// Gamescope's window size (`-W`/`-H`); 0 = Gamescope picks (the monitor's size).
    pub output_width: u32,
    pub output_height: u32,
    /// The size the game renders at (`-w`/`-h`); 0 = same as the window.
    pub game_width: u32,
    pub game_height: u32,
    /// Frame rate cap (`-r`); 0 = none (the HUD's cap is used when the HUD is on).
    pub fps_limit: u32,
    pub upscaler: Upscaler,
    /// 0 sharpest .. 20 softest; only for FSR and NIS.
    pub sharpness: u8,
    /// `--hdr-enabled`; only when the user ticks it.
    pub hdr: bool,
    /// `--force-grab-cursor`: always capture the mouse (helps with games that lose it).
    pub grab_cursor: bool,
    /// `--adaptive-sync`.
    pub adaptive_sync: bool,
}

impl Default for GamescopeSettings {
    fn default() -> Self {
        GamescopeSettings {
            enabled: false,
            window: WindowMode::Fullscreen,
            output_width: 0,
            output_height: 0,
            game_width: 0,
            game_height: 0,
            fps_limit: 0,
            upscaler: Upscaler::Default,
            sharpness: 2,
            hdr: false,
            grab_cursor: false,
            adaptive_sync: false,
        }
    }
}

impl GamescopeSettings {
    /// Fix values that came from a hand-edited file. A size needs both numbers.
    pub fn sanitized(mut self) -> GamescopeSettings {
        let size = |w: &mut u32, h: &mut u32| {
            *w = (*w).min(MAX_DIMENSION);
            *h = (*h).min(MAX_DIMENSION);
            if *w == 0 || *h == 0 {
                (*w, *h) = (0, 0);
            }
        };
        size(&mut self.output_width, &mut self.output_height);
        size(&mut self.game_width, &mut self.game_height);
        self.fps_limit = self.fps_limit.min(MAX_FPS);
        self.sharpness = self.sharpness.min(MAX_SHARPNESS);
        self
    }
}

/// A game's own choice about Gamescope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Override {
    /// Follow the global setting.
    #[default]
    UseGlobal,
    On,
    Off,
}

impl Override {
    pub const ALL: [Override; 3] = [Override::UseGlobal, Override::On, Override::Off];

    pub fn label(self) -> &'static str {
        match self {
            Override::UseGlobal => "Use the global setting",
            Override::On => "Always use Gamescope",
            Override::Off => "Never use Gamescope",
        }
    }

    /// Does the game run in Gamescope, given the global switch?
    pub fn resolve(self, global: bool) -> bool {
        match self {
            Override::UseGlobal => global,
            Override::On => true,
            Override::Off => false,
        }
    }
}

/// Per-game choices (`gamescope_games.json`): `appid -> Override`. Games that follow the global setting are not
/// stored.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GamescopeGames {
    pub apps: BTreeMap<String, Override>,
}

impl GamescopeGames {
    pub fn path() -> PathBuf {
        store::data_dir().join("gamescope_games.json")
    }

    pub fn load() -> GamescopeGames {
        store::load_json(&GamescopeGames::path()).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        store::save_json(&GamescopeGames::path(), self)
    }

    pub fn get(&self, appid: u32) -> Override {
        self.apps.get(&appid.to_string()).copied().unwrap_or_default()
    }

    pub fn set(&mut self, appid: u32, o: Override) {
        match o {
            Override::UseGlobal => self.apps.remove(&appid.to_string()),
            o => self.apps.insert(appid.to_string(), o),
        };
    }
}

// ---- Detection -----------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Detection {
    /// The native `gamescope` program.
    pub binary: Option<PathBuf>,
    /// Its version text (`gamescope --version`).
    pub version: Option<String>,
    /// `mangoapp` (MangoHud's Gamescope companion) is installed.
    pub mangoapp: bool,
    /// The Flatpak runtime extension `org.freedesktop.Platform.VulkanLayer.gamescope` is installed (what Flatpak
    /// Steam uses). Only detected: we do not start games through it.
    pub flatpak: bool,
}

impl Detection {
    pub fn installed(&self) -> bool {
        self.binary.is_some()
    }
}

pub const INSTALL_URL: &str = "https://github.com/ValveSoftware/gamescope";

/// How to install it, one line per package manager.
pub const INSTALL_HINTS: [(&str, &str); 3] = [
    ("Debian, Ubuntu", "sudo apt install gamescope"),
    ("Arch", "sudo pacman -S gamescope"),
    ("Fedora", "sudo dnf install gamescope"),
];

fn in_dirs(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// The version out of `gamescope --version`: the word after "version", else the first line.
pub fn parse_version(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|l| l.to_ascii_lowercase().contains("version"))?;
    let mut words = line.split_whitespace();
    while let Some(w) = words.next() {
        if w.eq_ignore_ascii_case("version") {
            return words.next().map(|v| v.trim_matches(|c| c == '(' || c == ')').to_string());
        }
    }
    Some(line.chars().take(60).collect())
}

/// Pure part of [`detect`]: the folders to look in. The version is filled in by [`detect`].
pub fn detect_in(path_dirs: &[PathBuf], flatpak_dirs: &[PathBuf]) -> Detection {
    Detection {
        binary: in_dirs(path_dirs, "gamescope"),
        version: None,
        mangoapp: in_dirs(path_dirs, "mangoapp").is_some(),
        flatpak: flatpak_dirs.iter().any(|d| d.is_dir()),
    }
}

/// Is Gamescope installed, and which version? Looks at `PATH` and runs `gamescope --version` once per run of
/// the program (that option only prints and exits; it never opens a session).
pub fn detect() -> Detection {
    use std::sync::OnceLock;
    static VERSION: OnceLock<Option<String>> = OnceLock::new();
    if !cfg!(unix) {
        return Detection::default();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let ext = "runtime/org.freedesktop.Platform.VulkanLayer.gamescope";
    let mut flatpak = vec![PathBuf::from("/var/lib/flatpak").join(ext)];
    if let Some(h) = std::env::var_os("HOME") {
        flatpak.push(PathBuf::from(h).join(".local/share/flatpak").join(ext));
    }
    let mut d = detect_in(&dirs, &flatpak);
    if let Some(bin) = &d.binary {
        d.version = VERSION
            .get_or_init(|| {
                let out = std::process::Command::new(bin)
                    .arg("--version")
                    .stdin(std::process::Stdio::null())
                    .output()
                    .ok()?;
                let text =
                    format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                parse_version(&text)
            })
            .clone();
    }
    d
}

/// The saved `gamescope` settings, read straight from `settings.json` (for the developer CLI).
pub fn settings_from_disk() -> GamescopeSettings {
    let v: Option<serde_json::Value> = store::load_json(&store::data_dir().join("settings.json"));
    v.and_then(|v| v.get("gamescope").cloned())
        .and_then(|g| serde_json::from_value::<GamescopeSettings>(g).ok())
        .unwrap_or_default()
        .sanitized()
}

// ---- The command ---------------------------------------------------------------------------------------------

/// The Gamescope options for `s` (everything before `--`). `mangoapp` adds `--mangoapp`; `fps` is the cap
/// to use (already resolved, 0 = none).
pub fn args(s: &GamescopeSettings, mangoapp: bool, fps: u32) -> Vec<String> {
    let s = s.clone().sanitized();
    let mut a: Vec<String> = Vec::new();
    let mut add = |items: &[&str]| a.extend(items.iter().map(|x| x.to_string()));
    if s.output_width > 0 {
        add(&["-W", &s.output_width.to_string(), "-H", &s.output_height.to_string()]);
    }
    if s.game_width > 0 {
        add(&["-w", &s.game_width.to_string(), "-h", &s.game_height.to_string()]);
    }
    match s.window {
        WindowMode::Fullscreen => add(&["-f"]),
        WindowMode::Borderless => add(&["-b"]),
        WindowMode::Windowed => {}
    }
    if fps > 0 {
        add(&["-r", &fps.to_string()]);
    }
    if let Some(f) = s.upscaler.flag() {
        add(&["-F", f]);
        if s.upscaler.sharpens() {
            add(&["--sharpness", &s.sharpness.to_string()]);
        }
    }
    if s.hdr {
        add(&["--hdr-enabled"]);
    }
    if s.grab_cursor {
        add(&["--force-grab-cursor"]);
    }
    if s.adaptive_sync {
        add(&["--adaptive-sync"]);
    }
    if mangoapp {
        add(&["--mangoapp"]);
    }
    a
}

/// What a launch needs to wrap a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub settings: GamescopeSettings,
    pub binary: PathBuf,
    /// Let Gamescope draw the HUD (`--mangoapp`). Only set when the HUD is on and `mangoapp` is installed.
    pub mangoapp: bool,
    /// The HUD's own frame cap, used for `-r` when Gamescope's setting has none (the HUD writes 0 then).
    pub hud_fps_limit: u32,
    /// Where the shim reports the display; `None` = no shim (only dry runs without a session).
    pub display_file: Option<PathBuf>,
}

impl Launch {
    /// The cap Gamescope gets.
    pub fn fps(&self) -> u32 {
        if self.settings.fps_limit > 0 { self.settings.fps_limit } else { self.hud_fps_limit }
    }
}

/// Decide a launch: `None` when `on` is false or Gamescope is not installed (the caller then starts the game
/// normally). `force_binary` lets a dry run show the command on a machine without it.
pub fn plan_launch(
    on: bool,
    s: &GamescopeSettings,
    hud: &HudSettings,
    appid: u32,
    det: &Detection,
    force_binary: bool,
) -> Option<Launch> {
    if !on || !cfg!(unix) {
        return None;
    }
    let binary = det.binary.clone().or_else(|| force_binary.then(|| PathBuf::from("gamescope")))?;
    Some(Launch {
        settings: s.clone().sanitized(),
        binary,
        mangoapp: hud.enabled() && det.mangoapp,
        hud_fps_limit: if hud.enabled() { hud.fps_limit } else { 0 },
        display_file: Some(display_file(appid)),
    })
}

/// The HUD settings a launch writes to MangoHud's config: its frame cap is zero when Gamescope does the capping.
pub fn hud_for(hud: &HudSettings, gamescope: bool) -> HudSettings {
    let mut h = hud.clone();
    if gamescope {
        h.fps_limit = 0;
    }
    h
}

/// The HUD without what Gamescope's `--mangoapp` replaces: no `MANGOHUD=1` (the layer in the game) and no
/// wrapper. The config file variable stays.
pub fn mangoapp_hud(hud: &Hud) -> Hud {
    let mut env = hud.env.clone();
    env.remove("MANGOHUD");
    Hud { env, wrapper: None }
}

/// Did the user's launch options already start Gamescope themselves (`gamescope -f -- %command%`)?
pub fn user_wrapped(user: &Parsed) -> bool {
    user.wrapper.iter().any(|w| w.rsplit('/').next() == Some("gamescope"))
}

/// The `sh` shim: write `$DISPLAY` (set by Gamescope for its child) to the file in `$1`, then run the rest.
const SHIM: &str = r#"printf '%s' "$DISPLAY" > "$1.tmp" && mv -f "$1.tmp" "$1"; shift; exec "$@""#;

/// Put Gamescope in front of the whole plan.
pub fn apply(plan: &mut LaunchPlan, l: &Launch) {
    let mut a = args(&l.settings, l.mangoapp, l.fps());
    a.push("--".into());
    let inner = std::iter::once(plan.program.to_string_lossy().into_owned()).chain(plan.args.drain(..));
    if let Some(file) = &l.display_file {
        a.extend(["/bin/sh", "-c", SHIM, "kettle-gamescope"].map(String::from));
        a.push(file.to_string_lossy().into_owned());
    }
    a.extend(inner);
    plan.program = l.binary.clone();
    plan.args = a;
    if l.settings.hdr {
        plan.env.entry("DXVK_HDR".into()).or_insert_with(|| "1".into());
    }
}

// ---- The display report --------------------------------------------------------------------------------------

/// `$XDG_RUNTIME_DIR/kettle-gamescope-<appid>.display` (our data folder when there is no runtime folder).
pub fn display_file(appid: u32) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(PathBuf::from);
    dir.unwrap_or_else(store::data_dir).join(format!("kettle-gamescope-{appid}.display"))
}

/// Remove an old report so a new launch never reads the display of the last one.
pub fn clear_display(appid: u32) {
    let _ = std::fs::remove_file(display_file(appid));
}

/// A display name like `:1` or `:1.0`; anything else is not trusted.
pub fn valid_display(s: &str) -> bool {
    let Some(rest) = s.strip_prefix(':') else { return false };
    let (num, screen) = rest.split_once('.').unwrap_or((rest, "0"));
    let digits = |t: &str| !t.is_empty() && t.len() <= 6 && t.bytes().all(|b| b.is_ascii_digit());
    digits(num) && digits(screen)
}

/// The display Gamescope made for the game, once the shim has reported it.
pub fn read_display(appid: u32) -> Option<String> {
    read_display_file(&display_file(appid))
}

pub fn read_display_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let d = text.trim();
    valid_display(d).then(|| d.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Runner;
    use std::collections::BTreeMap;

    fn plan() -> LaunchPlan {
        LaunchPlan {
            appid: 7,
            runner: Runner::Proton,
            program: PathBuf::from("/rt/run"),
            args: vec!["--".into(), "/proton".into(), "waitforexitandrun".into(), "/g/game.exe".into(), "-x".into()],
            cwd: PathBuf::from("/g"),
            env: BTreeMap::from([("STEAM_COMPAT_DATA_PATH".to_string(), "/cd".to_string())]),
            create_dirs: vec![],
        }
    }

    fn on() -> GamescopeSettings {
        GamescopeSettings { enabled: true, ..Default::default() }
    }

    fn launch(s: GamescopeSettings) -> Launch {
        Launch {
            settings: s,
            binary: PathBuf::from("/usr/bin/gamescope"),
            mangoapp: false,
            hud_fps_limit: 0,
            display_file: Some(PathBuf::from("/run/u/d.display")),
        }
    }

    fn strs(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn default_is_off_fullscreen_and_plain() {
        let s = GamescopeSettings::default();
        assert!(!s.enabled);
        assert_eq!(args(&s, false, 0), strs(&["-f"]));
    }

    #[test]
    fn every_option_becomes_its_flag() {
        let s = GamescopeSettings {
            enabled: true,
            window: WindowMode::Borderless,
            output_width: 2560,
            output_height: 1440,
            game_width: 1920,
            game_height: 1080,
            fps_limit: 60,
            upscaler: Upscaler::Fsr,
            sharpness: 5,
            hdr: true,
            grab_cursor: true,
            adaptive_sync: true,
        };
        assert_eq!(
            args(&s, true, s.fps_limit),
            strs(&[
                "-W",
                "2560",
                "-H",
                "1440",
                "-w",
                "1920",
                "-h",
                "1080",
                "-b",
                "-r",
                "60",
                "-F",
                "fsr",
                "--sharpness",
                "5",
                "--hdr-enabled",
                "--force-grab-cursor",
                "--adaptive-sync",
                "--mangoapp"
            ])
        );
    }

    #[test]
    fn windowed_and_upscalers() {
        let mut s = on();
        s.window = WindowMode::Windowed;
        assert!(args(&s, false, 0).is_empty());
        for (u, flag, sharp) in [
            (Upscaler::Nis, "nis", true),
            (Upscaler::Linear, "linear", false),
            (Upscaler::Nearest, "nearest", false),
            (Upscaler::Pixel, "pixel", false),
        ] {
            s.upscaler = u;
            let a = args(&s, false, 0);
            assert!(a.windows(2).any(|w| w == ["-F", flag]), "{flag}");
            assert_eq!(a.contains(&"--sharpness".to_string()), sharp, "{flag}");
        }
    }

    #[test]
    fn sizes_need_both_numbers_and_are_clamped() {
        let s = GamescopeSettings { output_width: 1920, game_height: 720, fps_limit: 99999, sharpness: 99, ..on() }
            .sanitized();
        assert_eq!((s.output_width, s.output_height, s.game_width, s.game_height), (0, 0, 0, 0));
        assert_eq!((s.fps_limit, s.sharpness), (1000, 20));
        assert!(!args(&s, false, 0).contains(&"-W".to_string()));
    }

    #[test]
    fn wraps_the_whole_command_behind_the_shim() {
        let mut p = plan();
        apply(&mut p, &launch(on()));
        assert_eq!(p.program, PathBuf::from("/usr/bin/gamescope"));
        assert_eq!(
            p.args[..9],
            strs(&["-f", "--", "/bin/sh", "-c", SHIM, "kettle-gamescope", "/run/u/d.display", "/rt/run", "--"])[..]
        );
        assert_eq!(p.args.last().map(String::as_str), Some("-x"));
        assert_eq!(p.args.len(), 7 + 1 + 5); // options + "--" ... shim words ... program + its 5 arguments
        assert_eq!(p.env.get("STEAM_COMPAT_DATA_PATH").map(String::as_str), Some("/cd"), "environment kept");
        assert_eq!(p.cwd, PathBuf::from("/g"));
    }

    #[test]
    fn user_wrappers_stay_inside() {
        // launch_opts::apply already put `gamemoderun` in front of the container command.
        let mut p = plan();
        let parsed = crate::launch_opts::parse("FOO=1 gamemoderun %command% -novid");
        p.args = parsed.args.clone();
        crate::launch_opts::apply(&mut p, &parsed);
        assert_eq!(p.program, PathBuf::from("gamemoderun"));
        apply(&mut p, &launch(on()));
        let at = p.args.iter().position(|a| a == "/run/u/d.display").unwrap();
        assert_eq!(p.args[at + 1], "gamemoderun", "the wrapper is the first thing the shim starts");
        assert_eq!(p.env.get("FOO").map(String::as_str), Some("1"));
        assert!(!user_wrapped(&parsed));
        assert!(user_wrapped(&crate::launch_opts::parse("/usr/bin/gamescope -f -- %command%")));
    }

    #[test]
    fn without_a_display_file_there_is_no_shim() {
        let mut p = plan();
        let mut l = launch(on());
        l.display_file = None;
        apply(&mut p, &l);
        assert_eq!(p.args[..3], strs(&["-f", "--", "/rt/run"])[..]);
    }

    #[test]
    fn frame_cap_goes_to_gamescope_once() {
        let mut l = launch(on());
        l.hud_fps_limit = 90;
        assert_eq!(l.fps(), 90, "the HUD's cap is used when Gamescope has none");
        l.settings.fps_limit = 60;
        assert_eq!(l.fps(), 60, "Gamescope's own cap wins");
        let hud = HudSettings { fps_limit: 90, ..HudSettings::default() };
        assert_eq!(hud_for(&hud, true).fps_limit, 0);
        assert_eq!(hud_for(&hud, false).fps_limit, 90);
    }

    #[test]
    fn mangoapp_replaces_the_layer() {
        let hud = Hud {
            env: BTreeMap::from([
                ("MANGOHUD".to_string(), "1".to_string()),
                ("MANGOHUD_CONFIGFILE".to_string(), "/c.conf".to_string()),
            ]),
            wrapper: Some(PathBuf::from("/usr/bin/mangohud")),
        };
        let m = mangoapp_hud(&hud);
        assert!(!m.env.contains_key("MANGOHUD"));
        assert!(m.env.contains_key("MANGOHUD_CONFIGFILE"));
        assert!(m.wrapper.is_none());
    }

    #[test]
    fn hdr_sets_the_dxvk_variable() {
        let mut p = plan();
        apply(&mut p, &launch(GamescopeSettings { hdr: true, ..on() }));
        assert_eq!(p.env.get("DXVK_HDR").map(String::as_str), Some("1"));
        let mut p = plan();
        apply(&mut p, &launch(on()));
        assert!(!p.env.contains_key("DXVK_HDR"));
    }

    #[test]
    fn plan_launch_needs_the_switch_and_the_program() {
        let det = Detection { binary: Some("/usr/bin/gamescope".into()), mangoapp: true, ..Default::default() };
        let hud = HudSettings { preset: crate::mangohud::Preset::FpsOnly, fps_limit: 75, ..HudSettings::default() };
        assert!(plan_launch(false, &on(), &hud, 7, &det, false).is_none());
        assert!(plan_launch(true, &on(), &hud, 7, &Detection::default(), false).is_none());
        let l = plan_launch(true, &on(), &hud, 7, &det, false).unwrap();
        assert!(l.mangoapp);
        assert_eq!(l.fps(), 75);
        let off = HudSettings::default();
        let l = plan_launch(true, &on(), &off, 7, &det, false).unwrap();
        assert!(!l.mangoapp);
        assert_eq!(l.fps(), 0);
        // A dry run may show the command without the program installed.
        let l = plan_launch(true, &on(), &off, 7, &Detection::default(), true).unwrap();
        assert_eq!(l.binary, PathBuf::from("gamescope"));
    }

    #[test]
    fn per_game_override_resolution() {
        assert!(Override::UseGlobal.resolve(true));
        assert!(!Override::UseGlobal.resolve(false));
        assert!(Override::On.resolve(false));
        assert!(!Override::Off.resolve(true));
        let mut g = GamescopeGames::default();
        assert_eq!(g.get(5), Override::UseGlobal);
        g.set(5, Override::On);
        g.set(6, Override::Off);
        assert_eq!((g.get(5), g.get(6), g.get(7)), (Override::On, Override::Off, Override::UseGlobal));
        g.set(5, Override::UseGlobal);
        assert!(!g.apps.contains_key("5"), "following the global setting stores nothing");
        let back: GamescopeGames = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn old_and_partial_settings_load() {
        let s: GamescopeSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, GamescopeSettings::default());
        let s: GamescopeSettings = serde_json::from_str(r#"{"enabled": true, "upscaler": "fsr"}"#).unwrap();
        assert!(s.enabled && s.window == WindowMode::Fullscreen && s.upscaler == Upscaler::Fsr);
        let back: GamescopeSettings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn versions_and_detection() {
        assert_eq!(parse_version("gamescope version 3.16.1 (gcc 14.2.1)\n").as_deref(), Some("3.16.1"));
        assert_eq!(parse_version("[gamescope] version 3.14.2-1\n").as_deref(), Some("3.14.2-1"));
        assert_eq!(parse_version("nothing useful"), None);
        let dir = std::env::temp_dir().join(format!("sr-gamescope-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!detect_in(std::slice::from_ref(&dir), &[]).installed());
        std::fs::write(dir.join("gamescope"), "").unwrap();
        std::fs::write(dir.join("mangoapp"), "").unwrap();
        let d = detect_in(std::slice::from_ref(&dir), std::slice::from_ref(&dir));
        assert!(d.installed() && d.mangoapp && d.flatpak);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn display_names_are_checked() {
        for ok in [":0", ":1", ":12.0"] {
            assert!(valid_display(ok), "{ok}");
        }
        for bad in ["", "1", ":", ":a", ":1;rm", "host:1", ":1.", ":99999999"] {
            assert!(!valid_display(bad), "{bad}");
        }
        let f = std::env::temp_dir().join(format!("sr-gs-display-{}", std::process::id()));
        std::fs::write(&f, ":3\n").unwrap();
        assert_eq!(read_display_file(&f).as_deref(), Some(":3"));
        std::fs::write(&f, "garbage").unwrap();
        assert_eq!(read_display_file(&f), None);
        std::fs::remove_file(&f).unwrap();
    }

    /// The shim really writes the display and runs the command (a stand-in for Gamescope sets `DISPLAY`).
    #[cfg(unix)]
    #[test]
    fn shim_reports_the_display_and_runs_the_command() {
        let f = std::env::temp_dir().join(format!("sr-gs-shim-{}", std::process::id()));
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", SHIM, "kettle-gamescope"])
            .arg(&f)
            .args(["/bin/echo", "ran"])
            .env("DISPLAY", ":7")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ran");
        assert_eq!(read_display_file(&f).as_deref(), Some(":7"));
        std::fs::remove_file(&f).unwrap();
    }
}
