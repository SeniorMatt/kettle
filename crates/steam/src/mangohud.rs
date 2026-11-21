//! The performance overlay (FPS counter and friends) through MangoHud.
//!
//! MangoHud draws inside the game's own window, like the official overlay does. We only write a config file per
//! game from our settings and add environment variables to the launch; we never start or wrap anything else.
//!
//! How it reaches the game:
//! * Proton, umu, DXVK, VKD3D-Proton: they render with Vulkan, so the MangoHud Vulkan layer is enough.
//!   `MANGOHUD=1` switches the implicit layer on and `MANGOHUD_CONFIGFILE` points at our file. No wrapper.
//! * Native Vulkan games: the same two variables.
//! * Native OpenGL games: the Vulkan layer does not see them. They need `libMangoHud.so` preloaded, which is what
//!   the `mangohud` wrapper script does (`LD_PRELOAD` plus `MANGOHUD_DLSYM=1` for some engines), so for native
//!   games the program is wrapped with it. Wine's own OpenGL translation (`PROTON_USE_WINED3D`) is not covered.
//! * Not covered: the Flatpak MangoHud extension (for Flatpak Steam) and Wayland-native-only OpenGL clients.
//!
//! If the user's own launch options already run `mangohud` or set a `MANGOHUD*` variable, theirs win and we add
//! nothing that would fight with it.

use crate::launch_opts::Parsed;
use crate::run::{LaunchPlan, Runner};
use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    #[default]
    Off,
    FpsOnly,
    FpsGraph,
    Full,
    Custom,
}

impl Preset {
    pub const ALL: [Preset; 5] = [Preset::Off, Preset::FpsOnly, Preset::FpsGraph, Preset::Full, Preset::Custom];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Off => "Off",
            Preset::FpsOnly => "FPS only",
            Preset::FpsGraph => "FPS + frame time graph",
            Preset::Full => "Full (CPU, GPU, RAM, VRAM)",
            Preset::Custom => "Custom",
        }
    }

    /// Short names for the overlay's quick settings.
    pub fn short(self) -> &'static str {
        match self {
            Preset::Off => "Off",
            Preset::FpsOnly => "FPS",
            Preset::FpsGraph => "Graph",
            Preset::Full => "Full",
            Preset::Custom => "Custom",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    TopCenter,
}

impl Position {
    pub const ALL: [Position; 5] =
        [Position::TopLeft, Position::TopRight, Position::BottomLeft, Position::BottomRight, Position::TopCenter];

    pub fn label(self) -> &'static str {
        match self {
            Position::TopLeft => "Top left",
            Position::TopRight => "Top right",
            Position::BottomLeft => "Bottom left",
            Position::BottomRight => "Bottom right",
            Position::TopCenter => "Top center",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Position::TopLeft => "top-left",
            Position::TopRight => "top-right",
            Position::BottomLeft => "bottom-left",
            Position::BottomRight => "bottom-right",
            Position::TopCenter => "top-center",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontSize {
    Small,
    #[default]
    Normal,
    Large,
}

impl FontSize {
    pub const ALL: [FontSize; 3] = [FontSize::Small, FontSize::Normal, FontSize::Large];

    pub fn label(self) -> &'static str {
        match self {
            FontSize::Small => "Small",
            FontSize::Normal => "Normal",
            FontSize::Large => "Large",
        }
    }

    fn px(self) -> u32 {
        match self {
            FontSize::Small => 18,
            FontSize::Normal => 24,
            FontSize::Large => 32,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextColor {
    #[default]
    White,
    Green,
    Yellow,
    Cyan,
    Orange,
}

impl TextColor {
    pub const ALL: [TextColor; 5] =
        [TextColor::White, TextColor::Green, TextColor::Yellow, TextColor::Cyan, TextColor::Orange];

    pub fn label(self) -> &'static str {
        match self {
            TextColor::White => "White",
            TextColor::Green => "Green",
            TextColor::Yellow => "Yellow",
            TextColor::Cyan => "Cyan",
            TextColor::Orange => "Orange",
        }
    }

    /// `RRGGBB`, as MangoHud wants it.
    fn hex(self) -> &'static str {
        match self {
            TextColor::White => "ffffff",
            TextColor::Green => "7bd957",
            TextColor::Yellow => "ffe14d",
            TextColor::Cyan => "57cbde",
            TextColor::Orange => "ff9f43",
        }
    }
}

/// What the HUD shows. Each field is one MangoHud option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Items {
    pub fps: bool,
    pub frametime_graph: bool,
    pub frame_ms: bool,
    pub cpu_load: bool,
    pub cpu_temp: bool,
    pub gpu_load: bool,
    pub gpu_temp: bool,
    pub gpu_clock: bool,
    pub ram: bool,
    pub vram: bool,
    pub battery: bool,
    pub time: bool,
    /// The game / engine name (`exec_name` and the engine version, e.g. DXVK).
    pub engine: bool,
    /// The Vulkan / OpenGL driver and the GPU name.
    pub driver: bool,
}

impl Default for Items {
    fn default() -> Self {
        Items::NONE.with(|i| i.fps = true)
    }
}

impl Items {
    pub const NONE: Items = Items {
        fps: false,
        frametime_graph: false,
        frame_ms: false,
        cpu_load: false,
        cpu_temp: false,
        gpu_load: false,
        gpu_temp: false,
        gpu_clock: false,
        ram: false,
        vram: false,
        battery: false,
        time: false,
        engine: false,
        driver: false,
    };

    fn with(mut self, f: impl FnOnce(&mut Items)) -> Items {
        f(&mut self);
        self
    }

    /// (label, field) for the custom toggles, in display order.
    pub fn fields(&mut self) -> [(&'static str, &mut bool); 14] {
        [
            ("FPS", &mut self.fps),
            ("Frame time graph", &mut self.frametime_graph),
            ("Frame time (ms)", &mut self.frame_ms),
            ("CPU load", &mut self.cpu_load),
            ("CPU temperature", &mut self.cpu_temp),
            ("GPU load", &mut self.gpu_load),
            ("GPU temperature", &mut self.gpu_temp),
            ("GPU clock", &mut self.gpu_clock),
            ("RAM", &mut self.ram),
            ("VRAM", &mut self.vram),
            ("Battery", &mut self.battery),
            ("Clock", &mut self.time),
            ("Game / engine name", &mut self.engine),
            ("Graphics driver", &mut self.driver),
        ]
    }
}

/// Our "Performance overlay" settings (`settings.json` -> `hud`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HudSettings {
    pub preset: Preset,
    /// Used when `preset` is [`Preset::Custom`].
    pub custom: Items,
    pub position: Position,
    pub font: FontSize,
    /// Background opacity, 0 to 100 percent.
    pub opacity: u8,
    pub color: TextColor,
    /// Frames per second cap; 0 = none.
    pub fps_limit: u32,
    /// MangoHud key combination that shows and hides the HUD, e.g. `Shift_L+F12`.
    pub toggle_key: String,
}

pub const DEFAULT_TOGGLE_KEY: &str = "Shift_L+F12";
pub const FPS_LIMIT_CHOICES: [u32; 5] = [0, 30, 60, 120, 144];
const MAX_FPS_LIMIT: u32 = 1000;

impl Default for HudSettings {
    fn default() -> Self {
        HudSettings {
            preset: Preset::Off,
            custom: Items::default(),
            position: Position::TopLeft,
            font: FontSize::Normal,
            opacity: 40,
            color: TextColor::White,
            fps_limit: 0,
            toggle_key: DEFAULT_TOGGLE_KEY.to_string(),
        }
    }
}

impl HudSettings {
    /// What is shown with the chosen preset.
    pub fn items(&self) -> Items {
        match self.preset {
            Preset::Off => Items::NONE,
            Preset::FpsOnly => Items::NONE.with(|i| i.fps = true),
            Preset::FpsGraph => Items::NONE.with(|i| {
                i.fps = true;
                i.frametime_graph = true;
            }),
            Preset::Full => Items::NONE.with(|i| {
                i.fps = true;
                i.frametime_graph = true;
                i.frame_ms = true;
                i.cpu_load = true;
                i.cpu_temp = true;
                i.gpu_load = true;
                i.gpu_temp = true;
                i.ram = true;
                i.vram = true;
            }),
            Preset::Custom => self.custom,
        }
    }

    /// Does this setting draw anything? (A frame rate cap alone does not need the HUD.)
    pub fn enabled(&self) -> bool {
        self.preset != Preset::Off
    }

    /// Fix values that came from a hand-edited file.
    pub fn sanitized(mut self) -> HudSettings {
        self.opacity = self.opacity.min(100);
        self.fps_limit = self.fps_limit.min(MAX_FPS_LIMIT);
        self.toggle_key = clean_key(&self.toggle_key);
        self
    }
}

/// Only letters, digits, `_` and `+` survive, so the value can never inject another config line.
fn clean_key(s: &str) -> String {
    let k: String = s.trim().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '+').collect();
    if k.is_empty() { DEFAULT_TOGGLE_KEY.to_string() } else { k }
}

/// The MangoHud config file text for `s`. Every option is written explicitly, because MangoHud turns some of
/// them on by default (fps, frame timing, CPU and GPU stats) when a file does not mention them.
pub fn config_text(s: &HudSettings) -> String {
    let s = s.clone().sanitized();
    let i = s.items();
    let b = |on: bool| u8::from(on);
    let mut out = String::from("# Written by Kettle. Changes are overwritten at the next launch.\n");
    let mut line = |k: &str, v: String| {
        out.push_str(k);
        out.push('=');
        out.push_str(&v);
        out.push('\n');
    };
    line("legacy_layout", "0".into());
    line("fps", b(i.fps).to_string());
    line("frametime", b(i.frame_ms).to_string());
    line("frame_timing", b(i.frametime_graph).to_string());
    // `cpu_stats` / `gpu_stats` are the load switches; the temperature and clock options need them on.
    line("cpu_stats", b(i.cpu_load || i.cpu_temp).to_string());
    line("cpu_temp", b(i.cpu_temp).to_string());
    line("gpu_stats", b(i.gpu_load || i.gpu_temp || i.gpu_clock).to_string());
    line("gpu_temp", b(i.gpu_temp).to_string());
    line("gpu_core_clock", b(i.gpu_clock).to_string());
    line("ram", b(i.ram).to_string());
    line("vram", b(i.vram).to_string());
    line("battery", b(i.battery).to_string());
    line("time", b(i.time).to_string());
    line("exec_name", b(i.engine).to_string());
    line("engine_version", b(i.engine).to_string());
    line("vulkan_driver", b(i.driver).to_string());
    line("gpu_name", b(i.driver).to_string());
    line("position", s.position.key().to_string());
    line("font_size", s.font.px().to_string());
    line("background_alpha", format!("{:.2}", f32::from(s.opacity) / 100.0));
    line("text_color", s.color.hex().to_string());
    line("fps_limit", s.fps_limit.to_string());
    line("toggle_hud", s.toggle_key);
    out
}

// ---- Detecting MangoHud ---------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Detection {
    /// The `mangohud` wrapper script.
    pub wrapper: Option<PathBuf>,
    /// The library or the Vulkan layer manifest is installed.
    pub layer: bool,
}

impl Detection {
    pub fn installed(&self) -> bool {
        self.wrapper.is_some() || self.layer
    }
}

pub const INSTALL_URL: &str = "https://github.com/flightlessmango/MangoHud";

/// How to install it, one line per package manager.
pub const INSTALL_HINTS: [(&str, &str); 3] = [
    ("Debian, Ubuntu", "sudo apt install mangohud"),
    ("Arch", "sudo pacman -S mangohud"),
    ("Fedora", "sudo dnf install mangohud"),
];

/// Pure part of [`detect`]: look in the given folders.
pub fn detect_in(path_dirs: &[PathBuf], lib_dirs: &[PathBuf], layer_dirs: &[PathBuf]) -> Detection {
    let wrapper = path_dirs.iter().map(|d| d.join("mangohud")).find(|p| p.is_file());
    let lib = lib_dirs.iter().any(|d| d.is_dir());
    let manifest = layer_dirs.iter().any(|d| {
        std::fs::read_dir(d).is_ok_and(|rd| {
            rd.flatten().any(|e| e.file_name().to_string_lossy().to_ascii_lowercase().starts_with("mangohud"))
        })
    });
    Detection { wrapper, layer: lib || manifest }
}

/// Is MangoHud installed natively? (Looks at `PATH`, `/usr/lib*/mangohud` and the Vulkan layer folders.)
pub fn detect() -> Detection {
    if !cfg!(unix) {
        return Detection::default();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path_dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    let mut libs: Vec<PathBuf> = ["/usr/lib/mangohud", "/usr/lib64/mangohud", "/usr/lib/x86_64-linux-gnu/mangohud"]
        .iter()
        .map(PathBuf::from)
        .collect();
    // `/usr/lib*/mangohud` for other multiarch names.
    if let Ok(rd) = std::fs::read_dir("/usr") {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with("lib") {
                libs.push(e.path().join("mangohud"));
            }
        }
    }
    let layers: Vec<PathBuf> =
        ["/usr/share/vulkan/implicit_layer.d", "/etc/vulkan/implicit_layer.d"].iter().map(PathBuf::from).collect();
    detect_in(&path_dirs, &libs, &layers)
}

// ---- Launch ---------------------------------------------------------------------------------------------

/// What a launch adds for the HUD.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Hud {
    pub env: BTreeMap<String, String>,
    /// For native games: run the game through this (`mangohud`), so OpenGL is covered too.
    pub wrapper: Option<PathBuf>,
}

/// The variables for a config file at `config`.
pub fn launch_env(config: &Path) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("MANGOHUD".to_string(), "1".to_string()),
        ("MANGOHUD_CONFIGFILE".to_string(), config.to_string_lossy().into_owned()),
    ])
}

/// Where the config of one game lives, below `dir`.
pub fn config_path_in(dir: &Path, appid: u32) -> PathBuf {
    dir.join("mangohud").join(format!("{appid}.conf"))
}

/// Write the config for `appid` (mode 0600) and say what to add to the launch. `Ok(None)` when the HUD is off
/// or MangoHud is not installed. An frame rate cap alone with the HUD "off" is not applied: the setting is
/// off, so nothing about the game changes.
pub fn prepare(s: &HudSettings, appid: u32) -> anyhow::Result<Option<Hud>> {
    if !s.enabled() {
        return Ok(None);
    }
    let found = detect();
    if !found.installed() {
        return Ok(None);
    }
    let path = config_path_in(&store::data_dir(), appid);
    store::write_private(&path, config_text(s).as_bytes())?;
    Ok(Some(Hud { env: launch_env(&path), wrapper: found.wrapper }))
}

/// The saved `hud` settings, read straight from `settings.json` (for the developer CLI, which has no client).
pub fn settings_from_disk() -> HudSettings {
    let v: Option<serde_json::Value> = store::load_json(&store::data_dir().join("settings.json"));
    v.and_then(|v| v.get("hud").cloned())
        .and_then(|h| serde_json::from_value::<HudSettings>(h).ok())
        .unwrap_or_default()
        .sanitized()
}

/// Do the user's own launch options already run MangoHud (a `mangohud` wrapper or a `MANGOHUD*` variable)?
pub fn user_owns(user: &Parsed) -> bool {
    user.wrapper.iter().any(|w| w.rsplit('/').next() == Some("mangohud"))
        || user.env.iter().any(|(k, _)| k.starts_with("MANGOHUD"))
}

/// Add the HUD to a plan. The user's own launch options win: a `mangohud` wrapper or any `MANGOHUD*` variable
/// there means we add nothing, and an existing variable is never overwritten.
pub fn apply(plan: &mut LaunchPlan, hud: &Hud, user: &Parsed) {
    if user_owns(user) {
        return;
    }
    for (k, v) in &hud.env {
        plan.env.entry(k.clone()).or_insert_with(|| v.clone());
    }
    if plan.runner == Runner::Native
        && let Some(w) = &hud.wrapper
    {
        let program = std::mem::replace(&mut plan.program, w.clone());
        plan.args.insert(0, program.to_string_lossy().into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(text: &str, key: &str) -> Option<String> {
        text.lines().find_map(|l| l.strip_prefix(&format!("{key}=")).map(str::to_string))
    }

    fn with(preset: Preset) -> HudSettings {
        HudSettings { preset, ..HudSettings::default() }
    }

    fn plan(runner: Runner) -> LaunchPlan {
        LaunchPlan {
            appid: 7,
            runner,
            program: PathBuf::from("/games/g/run.sh"),
            args: vec!["-x".into()],
            cwd: PathBuf::from("/games/g"),
            env: BTreeMap::new(),
            create_dirs: vec![],
        }
    }

    fn hud() -> Hud {
        Hud { env: launch_env(Path::new("/d/mangohud/7.conf")), wrapper: Some(PathBuf::from("/usr/bin/mangohud")) }
    }

    #[test]
    fn fps_only_preset() {
        let t = config_text(&with(Preset::FpsOnly));
        assert_eq!(value(&t, "fps").as_deref(), Some("1"));
        for off in ["frame_timing", "frametime", "cpu_stats", "gpu_stats", "ram", "vram", "battery", "time"] {
            assert_eq!(value(&t, off).as_deref(), Some("0"), "{off}");
        }
    }

    #[test]
    fn graph_preset_adds_only_the_graph() {
        let t = config_text(&with(Preset::FpsGraph));
        assert_eq!(value(&t, "fps").as_deref(), Some("1"));
        assert_eq!(value(&t, "frame_timing").as_deref(), Some("1"));
        assert_eq!(value(&t, "cpu_stats").as_deref(), Some("0"));
    }

    #[test]
    fn full_preset_shows_the_hardware() {
        let t = config_text(&with(Preset::Full));
        for on in ["fps", "frame_timing", "cpu_stats", "cpu_temp", "gpu_stats", "gpu_temp", "ram", "vram"] {
            assert_eq!(value(&t, on).as_deref(), Some("1"), "{on}");
        }
        assert_eq!(value(&t, "battery").as_deref(), Some("0"));
    }

    #[test]
    fn off_preset_shows_nothing_and_is_not_enabled() {
        let s = with(Preset::Off);
        assert!(!s.enabled());
        assert_eq!(s.items(), Items::NONE);
        assert_eq!(value(&config_text(&s), "fps").as_deref(), Some("0"));
    }

    #[test]
    fn custom_toggles_map_to_options() {
        let mut s = with(Preset::Custom);
        s.custom = Items::NONE.with(|i| {
            i.gpu_clock = true;
            i.time = true;
            i.engine = true;
            i.driver = true;
            i.battery = true;
        });
        let t = config_text(&s);
        assert_eq!(value(&t, "fps").as_deref(), Some("0"));
        assert_eq!(value(&t, "gpu_core_clock").as_deref(), Some("1"));
        assert_eq!(value(&t, "gpu_stats").as_deref(), Some("1"), "the clock needs the GPU stats switch");
        assert_eq!(value(&t, "gpu_temp").as_deref(), Some("0"));
        assert_eq!(value(&t, "time").as_deref(), Some("1"));
        assert_eq!(value(&t, "exec_name").as_deref(), Some("1"));
        assert_eq!(value(&t, "engine_version").as_deref(), Some("1"));
        assert_eq!(value(&t, "vulkan_driver").as_deref(), Some("1"));
        assert_eq!(value(&t, "battery").as_deref(), Some("1"));
        assert_eq!(value(&t, "cpu_stats").as_deref(), Some("0"));
    }

    #[test]
    fn look_options() {
        let s = HudSettings {
            position: Position::BottomRight,
            font: FontSize::Large,
            opacity: 25,
            color: TextColor::Green,
            fps_limit: 144,
            ..with(Preset::FpsOnly)
        };
        let t = config_text(&s);
        assert_eq!(value(&t, "position").as_deref(), Some("bottom-right"));
        assert_eq!(value(&t, "font_size").as_deref(), Some("32"));
        assert_eq!(value(&t, "background_alpha").as_deref(), Some("0.25"));
        assert_eq!(value(&t, "text_color").as_deref(), Some("7bd957"));
        assert_eq!(value(&t, "fps_limit").as_deref(), Some("144"));
        assert_eq!(value(&t, "toggle_hud").as_deref(), Some("Shift_L+F12"));
        assert_eq!(value(&config_text(&with(Preset::FpsOnly)), "position").as_deref(), Some("top-left"));
        let c = HudSettings { position: Position::TopCenter, ..with(Preset::FpsOnly) };
        assert_eq!(value(&config_text(&c), "position").as_deref(), Some("top-center"));
    }

    #[test]
    fn hand_edited_values_are_cleaned() {
        let s = HudSettings {
            opacity: 250,
            fps_limit: 99_999,
            toggle_key: "F9\nfps_limit=1".into(),
            ..with(Preset::FpsOnly)
        };
        let t = config_text(&s);
        assert_eq!(value(&t, "background_alpha").as_deref(), Some("1.00"));
        assert_eq!(value(&t, "fps_limit").as_deref(), Some("1000"));
        assert_eq!(value(&t, "toggle_hud").as_deref(), Some("F9fps_limit1"));
        assert_eq!(t.lines().filter(|l| l.starts_with("fps_limit=")).count(), 1);
        assert_eq!(clean_key("  "), DEFAULT_TOGGLE_KEY);
    }

    #[test]
    fn old_settings_files_load() {
        let s: HudSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(s, HudSettings::default());
        let s: HudSettings = serde_json::from_str(r#"{"preset":"full","position":"top_right"}"#).unwrap();
        assert_eq!(s.preset, Preset::Full);
        assert_eq!(s.position, Position::TopRight);
        assert_eq!(s.opacity, 40);
    }

    #[test]
    fn env_and_config_path() {
        let p = config_path_in(Path::new("/data"), 730);
        assert_eq!(p, PathBuf::from("/data/mangohud/730.conf"));
        let env = launch_env(&p);
        assert_eq!(env["MANGOHUD"], "1");
        assert_eq!(env["MANGOHUD_CONFIGFILE"], "/data/mangohud/730.conf");
        assert_eq!(env.len(), 2);
    }

    #[test]
    fn native_games_get_the_wrapper_and_proton_games_do_not() {
        let mut p = plan(Runner::Native);
        apply(&mut p, &hud(), &Parsed::default());
        assert_eq!(p.env["MANGOHUD"], "1");
        assert_eq!(p.program, PathBuf::from("/usr/bin/mangohud"));
        assert_eq!(p.args, vec!["/games/g/run.sh", "-x"]);

        let mut p = plan(Runner::Proton);
        apply(&mut p, &hud(), &Parsed::default());
        assert_eq!(p.env["MANGOHUD_CONFIGFILE"], "/d/mangohud/7.conf");
        assert_eq!(p.program, PathBuf::from("/games/g/run.sh"));
        assert_eq!(p.args, vec!["-x"]);
    }

    #[test]
    fn the_users_own_mangohud_wins() {
        let mut p = plan(Runner::Native);
        let user = Parsed { wrapper: vec!["/usr/bin/mangohud".into()], ..Parsed::default() };
        apply(&mut p, &hud(), &user);
        assert!(p.env.is_empty());
        assert_eq!(p.program, PathBuf::from("/games/g/run.sh"));

        let mut p = plan(Runner::Umu);
        let user = Parsed { env: vec![("MANGOHUD_CONFIG".into(), "fps".into())], ..Parsed::default() };
        apply(&mut p, &hud(), &user);
        assert!(p.env.is_empty());

        // An env value already on the plan is kept.
        let mut p = plan(Runner::Umu);
        p.env.insert("MANGOHUD".into(), "0".into());
        apply(&mut p, &hud(), &Parsed::default());
        assert_eq!(p.env["MANGOHUD"], "0");
        assert_eq!(p.env["MANGOHUD_CONFIGFILE"], "/d/mangohud/7.conf");
    }

    #[test]
    fn detection_in_folders() {
        let root = std::env::temp_dir().join(format!("sr-mangohud-{}", std::process::id()));
        let (bin, lib, layers) = (root.join("bin"), root.join("lib/mangohud"), root.join("layers"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&layers).unwrap();
        let none = detect_in(std::slice::from_ref(&bin), std::slice::from_ref(&lib), std::slice::from_ref(&layers));
        assert!(!none.installed());
        std::fs::write(bin.join("mangohud"), "#!/bin/sh\n").unwrap();
        let d = detect_in(std::slice::from_ref(&bin), std::slice::from_ref(&lib), std::slice::from_ref(&layers));
        assert_eq!(d.wrapper, Some(bin.join("mangohud")));
        assert!(d.installed());
        std::fs::write(layers.join("MangoHud.x86_64.json"), "{}").unwrap();
        assert!(detect_in(&[], &[lib], &[layers]).layer);
        let _ = std::fs::remove_dir_all(&root);
    }
}
