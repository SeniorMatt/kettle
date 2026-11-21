//! Non-Steam games ("Add a Game"): any program the user picks, listed in the library next to the Steam games.
//!
//! They are kept in our own data dir (`non_steam.json`); Steam's `shortcuts.vdf` is only ever read (to offer an
//! import) and never written. Each game gets an id with the top bit set, like Steam's own shortcuts, so it can't
//! clash with a real app id. A Linux program starts as it is; a Windows one (`.exe`) goes through the default
//! compatibility tool with a prefix of its own. Play time is counted like for the other games.

use super::{Client, Event, PlayProblem};
use serde::{Deserialize, Serialize};
use sr_model::{AppId, Game, InstallState};
use sr_steam::{local, run, store};
use std::path::{Path, PathBuf};

pub const MAX_NAME: usize = 100;
pub const MAX_ARGS: usize = 1000;
/// How many non-Steam games are kept.
pub const MAX_GAMES: usize = 2000;
/// Ids of non-Steam games have this bit set.
pub const ID_BIT: u32 = 0x8000_0000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NonSteamGame {
    pub id: u32,
    pub name: String,
    pub exe: PathBuf,
    /// Arguments as one text (split like launch options).
    #[serde(default)]
    pub args: String,
    /// A PNG/JPEG icon, e.g. from the `.desktop` file it was picked from.
    #[serde(default)]
    pub icon: Option<PathBuf>,
    /// Read from Steam's shortcuts instead of added by hand.
    #[serde(default)]
    pub imported: bool,
}

impl NonSteamGame {
    /// The folder the program lives in (shown as the "install" folder).
    pub fn folder(&self) -> PathBuf {
        self.exe.parent().map(Path::to_path_buf).unwrap_or_default()
    }

    /// A Windows program (needs Proton).
    pub fn is_windows(&self) -> bool {
        let n = self.exe.to_string_lossy().to_ascii_lowercase();
        [".exe", ".bat", ".cmd", ".com", ".msi", ".lnk"].iter().any(|x| n.ends_with(x))
    }
}

/// What the UI may read.
#[derive(Debug, Clone, Default)]
pub struct NonSteamState {
    pub games: Vec<NonSteamGame>,
}

impl NonSteamState {
    pub fn get(&self, app: AppId) -> Option<&NonSteamGame> {
        self.games.iter().find(|g| g.id == app.0)
    }

    pub fn contains(&self, app: AppId) -> bool {
        self.get(app).is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NonSteamCmd {
    /// Add a program. An empty name is taken from the file name. `icon` must be a PNG/JPEG file.
    Add { name: String, exe: PathBuf, args: String, icon: Option<PathBuf> },
    /// Take a game off the list (the program itself stays where it is).
    Remove(AppId),
    /// Read the shortcuts the official client has (read-only) and add them.
    ImportSteam,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct File {
    #[serde(default)]
    games: Vec<NonSteamGame>,
}

fn path() -> PathBuf {
    store::data_dir().join("non_steam.json")
}

impl NonSteamState {
    pub(crate) fn load() -> NonSteamState {
        let file: File = store::load_json(&path()).unwrap_or_default();
        let mut games: Vec<NonSteamGame> = Vec::new();
        for mut g in file.games {
            g.id |= ID_BIT;
            if !games.iter().any(|x| x.id == g.id) && !g.name.trim().is_empty() {
                games.push(g);
            }
        }
        games.truncate(MAX_GAMES);
        NonSteamState { games }
    }
}

// ---- ids ----------------------------------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Steam's formula for a shortcut's app id: CRC-32 of the quoted program and the name, top bit set.
pub fn shortcut_id(exe: &str, name: &str) -> u32 {
    crc32(format!("{exe}{name}").as_bytes()) | ID_BIT
}

// ---- checking input -----------------------------------------------------------------------------------------

fn clean_text(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

fn is_image(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["png", "jpg", "jpeg"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Check what the user entered and make the record (the id is set by the caller).
pub fn make_game(name: &str, exe: &Path, args: &str, icon: Option<&Path>) -> Result<NonSteamGame, String> {
    if !exe.is_absolute() {
        return Err("Pick the program with its full path.".into());
    }
    if !exe.is_file() {
        return Err(format!("{} is not a file.", exe.display()));
    }
    let name = match clean_text(name, MAX_NAME) {
        n if n.is_empty() => exe.file_stem().map(|s| clean_text(&s.to_string_lossy(), MAX_NAME)).unwrap_or_default(),
        n => n,
    };
    if name.is_empty() {
        return Err("Give the game a name.".into());
    }
    if args.len() > MAX_ARGS || args.chars().any(|c| c == '\0' || c == '\n' || c == '\r') {
        return Err("The arguments are too long or have a line break.".into());
    }
    let icon = icon.filter(|p| p.is_absolute() && p.is_file() && is_image(p)).map(Path::to_path_buf);
    Ok(NonSteamGame {
        id: shortcut_id(&format!("\"{}\"", exe.display()), &name),
        name,
        exe: exe.to_path_buf(),
        args: args.trim().to_string(),
        icon,
        imported: false,
    })
}

// ---- .desktop files -----------------------------------------------------------------------------------------

/// What a `.desktop` file says about how to start an app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopEntry {
    pub name: String,
    pub exe: PathBuf,
    pub args: String,
    pub icon: Option<PathBuf>,
}

/// Read `Name`, `Exec` and `Icon` from the `[Desktop Entry]` section.
pub fn parse_desktop(text: &str) -> Option<DesktopEntry> {
    let (mut name, mut exec, mut icon, mut in_entry) = (None, None, None, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
        } else if in_entry && let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "Name" if name.is_none() => name = Some(v.trim().to_string()),
                "Exec" if exec.is_none() => exec = Some(v.trim().to_string()),
                "Icon" if icon.is_none() => icon = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    let mut parts = run::split_args(&exec?);
    // Field codes (%f, %U, ...) stand for files the launcher would pass; "%%" is a literal percent sign.
    parts.retain(|p| !(p.len() == 2 && p.starts_with('%') && p != "%%"));
    let parts: Vec<String> = parts.into_iter().map(|p| p.replace("%%", "%")).collect();
    let (program, rest) = parts.split_first()?;
    let exe = resolve_program(program)?;
    let args = rest.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ");
    Some(DesktopEntry { name: clean_text(&name?, MAX_NAME), exe, args, icon: icon.as_deref().and_then(find_icon) })
}

fn quote_arg(a: &str) -> String {
    if !a.is_empty() && a.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:=+,@%".contains(c)) {
        a.to_string()
    } else if !a.contains('"') {
        format!("\"{a}\"")
    } else {
        format!("'{a}'")
    }
}

/// An absolute program as it is; a bare name is searched in `$PATH`.
fn resolve_program(p: &str) -> Option<PathBuf> {
    let path = Path::new(p);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    if p.contains('/') {
        return None;
    }
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).map(|d| d.join(p)).find(|c| c.is_file())
}

/// Read a `.desktop` file (at most 64 KB).
pub fn read_desktop_file(path: &Path) -> Result<DesktopEntry, String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(64 * 1024).read_to_string(&mut text))
        .map_err(|e| format!("Couldn't read {}: {e}", path.display()))?;
    parse_desktop(&text).ok_or_else(|| "That file doesn't say which program to start.".to_string())
}

/// An icon file for `Icon=`: an absolute path, or a name looked up in the usual icon folders (PNG only).
pub fn find_icon(icon: &str) -> Option<PathBuf> {
    let p = Path::new(icon);
    if p.is_absolute() {
        return (p.is_file() && is_image(p)).then(|| p.to_path_buf());
    }
    if icon.is_empty() || icon.contains('/') || icon.contains("..") {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut roots: Vec<PathBuf> = vec![PathBuf::from("/usr/share/icons/hicolor"), PathBuf::from("/usr/share/pixmaps")];
    if let Some(h) = &home {
        roots.insert(0, h.join(".local/share/icons/hicolor"));
    }
    for root in &roots {
        if root.ends_with("pixmaps") {
            let f = root.join(format!("{icon}.png"));
            if f.is_file() {
                return Some(f);
            }
            continue;
        }
        for size in ["256x256", "128x128", "96x96", "64x64", "48x48", "32x32"] {
            let f = root.join(size).join("apps").join(format!("{icon}.png"));
            if f.is_file() {
                return Some(f);
            }
        }
    }
    None
}

/// The apps in the system's `.desktop` files (name and file), for "Add from an installed app". Games and other
/// visible applications; sorted by name; at most 500.
pub fn installed_desktop_apps() -> Vec<(String, PathBuf)> {
    let mut dirs = vec![PathBuf::from("/usr/share/applications"), PathBuf::from("/usr/local/share/applications")];
    if let Some(d) = super::shortcuts::applications_dir() {
        dirs.push(d);
    }
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for f in rd.flatten() {
            let p = f.path();
            if p.extension().is_none_or(|e| e != "desktop") {
                continue;
            }
            // Kettle's own game shortcuts would only list games twice.
            if p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("kettle")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            if text.len() > 64 * 1024 || text.lines().any(|l| l.trim() == "NoDisplay=true") {
                continue;
            }
            if let Some(name) = text.lines().find_map(|l| l.strip_prefix("Name=")) {
                out.push((clean_text(name, MAX_NAME), p));
            }
        }
    }
    out.sort_by_key(|a| a.0.to_lowercase());
    out.truncate(500);
    out
}

// ---- Steam's shortcuts.vdf (binary KeyValues, read-only) ------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Str(String),
    Int(u32),
    Map(Vec<(String, Val)>),
}

fn cstr(b: &[u8], pos: &mut usize) -> Option<String> {
    let rest = b.get(*pos..)?;
    let end = rest.iter().position(|&c| c == 0)?;
    let s = String::from_utf8_lossy(&rest[..end]).into_owned();
    *pos += end + 1;
    Some(s)
}

fn parse_map(b: &[u8], pos: &mut usize, depth: usize) -> Option<Vec<(String, Val)>> {
    if depth > 8 {
        return None;
    }
    let mut out = Vec::new();
    loop {
        let kind = *b.get(*pos)?;
        *pos += 1;
        if kind == 0x08 {
            return Some(out);
        }
        let key = cstr(b, pos)?;
        let val = match kind {
            0x00 => Val::Map(parse_map(b, pos, depth + 1)?),
            0x01 => Val::Str(cstr(b, pos)?),
            0x02 => {
                let n = b.get(*pos..*pos + 4)?;
                *pos += 4;
                Val::Int(u32::from_le_bytes([n[0], n[1], n[2], n[3]]))
            }
            _ => return None,
        };
        out.push((key, val));
        if out.len() > 100_000 {
            return None;
        }
    }
}

fn field<'a>(m: &'a [(String, Val)], name: &str) -> Option<&'a Val> {
    m.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v)
}

fn text_field(m: &[(String, Val)], name: &str) -> String {
    match field(m, name) {
        Some(Val::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

/// The shortcuts in the bytes of a `shortcuts.vdf`.
pub fn parse_shortcuts(bytes: &[u8]) -> Vec<NonSteamGame> {
    let mut pos = 0;
    let Some(top) = parse_map(bytes, &mut pos, 0) else { return Vec::new() };
    let Some(Val::Map(entries)) = field(&top, "shortcuts") else { return Vec::new() };
    let mut out = Vec::new();
    for (_, v) in entries {
        let Val::Map(m) = v else { continue };
        let exe_raw = text_field(m, "exe");
        let exe = exe_raw.trim().trim_matches('"').to_string();
        let name = clean_text(&text_field(m, "appname"), MAX_NAME);
        if exe.is_empty() || name.is_empty() || !Path::new(&exe).is_absolute() {
            continue;
        }
        let id = match field(m, "appid") {
            Some(Val::Int(n)) if n & ID_BIT != 0 => *n,
            _ => shortcut_id(&exe_raw, &name),
        };
        let icon = Some(text_field(m, "icon")).filter(|i| !i.is_empty()).map(PathBuf::from);
        out.push(NonSteamGame {
            id,
            name,
            exe: PathBuf::from(exe),
            args: clean_text(&text_field(m, "launchoptions"), MAX_ARGS),
            icon: icon.filter(|p| p.is_absolute() && is_image(p)),
            imported: true,
        });
    }
    out
}

/// Every `userdata/*/config/shortcuts.vdf` of the Steam installs on this machine, read only.
pub fn read_steam_shortcuts() -> Vec<NonSteamGame> {
    let mut out: Vec<NonSteamGame> = Vec::new();
    for root in local::steam_roots() {
        let Ok(users) = std::fs::read_dir(root.join("userdata")) else { continue };
        for user in users.flatten() {
            let file = user.path().join("config/shortcuts.vdf");
            // A shortcuts file is a few KB; refuse anything absurd.
            if std::fs::metadata(&file).is_ok_and(|m| m.len() <= 8 << 20)
                && let Ok(bytes) = std::fs::read(&file)
            {
                for g in parse_shortcuts(&bytes) {
                    if !out.iter().any(|x| x.id == g.id) {
                        out.push(g);
                    }
                }
            }
        }
    }
    out
}

// ---- the client side ----------------------------------------------------------------------------------------

impl Client {
    pub(crate) fn non_steam_cmd(&mut self, cmd: NonSteamCmd) {
        match cmd {
            NonSteamCmd::Add { name, exe, args, icon } => match make_game(&name, &exe, &args, icon.as_deref()) {
                Ok(g) => self.add_non_steam(g),
                Err(e) => self.state.notice = Some(e),
            },
            NonSteamCmd::Remove(app) => self.remove_non_steam(app),
            NonSteamCmd::ImportSteam => {
                let (tx, wake) = (self.tx.clone(), self.wake.clone());
                self.rt.spawn_blocking(move || {
                    let _ = tx.blocking_send(Event::SteamShortcuts(read_steam_shortcuts()));
                    wake();
                });
            }
        }
    }

    fn save_non_steam(&mut self) {
        let file = File { games: self.state.non_steam.games.clone() };
        if let Err(e) = store::save_json(&path(), &file) {
            self.state.notice = Some(format!("Couldn't save the game list: {e:#}"));
        }
    }

    fn add_non_steam(&mut self, mut g: NonSteamGame) {
        let list = &mut self.state.non_steam.games;
        if list.len() >= MAX_GAMES {
            self.state.notice = Some("That is more non-Steam games than Kettle keeps.".into());
            return;
        }
        // Same program and name again: replace it. Otherwise find a free id.
        list.retain(|x| x.id != g.id);
        while list.iter().any(|x| x.id == g.id) {
            g.id = (g.id.wrapping_add(1)) | ID_BIT;
        }
        self.state.notice = Some(format!("{} was added to your library.", g.name));
        list.push(g);
        self.save_non_steam();
        self.merge_installed();
    }

    fn remove_non_steam(&mut self, app: AppId) {
        let before = self.state.non_steam.games.len();
        self.state.non_steam.games.retain(|g| g.id != app.0);
        if self.state.non_steam.games.len() == before {
            return;
        }
        self.state.games.retain(|g| g.app_id != app);
        self.base_play.remove(&app.0);
        if self.state.selected == Some(app) {
            self.state.selected = None;
        }
        self.save_non_steam();
        self.merge_installed();
    }

    /// Steam's shortcuts have been read: add the ones we don't have.
    pub(crate) fn steam_shortcuts_read(&mut self, found: Vec<NonSteamGame>) {
        let mut added = 0;
        for g in found {
            if self.state.non_steam.games.len() < MAX_GAMES && !self.state.non_steam.games.iter().any(|x| x.id == g.id)
            {
                self.state.non_steam.games.push(g);
                added += 1;
            }
        }
        self.state.notice = Some(match added {
            0 => "No new shortcuts found in the Steam client.".to_string(),
            1 => "Imported 1 shortcut from the Steam client.".to_string(),
            n => format!("Imported {n} shortcuts from the Steam client."),
        });
        if added > 0 {
            self.save_non_steam();
            self.merge_installed();
        }
    }

    /// Put the non-Steam games into `state.games` (called from `merge_installed`, before play time is added).
    pub(crate) fn add_non_steam_games(&mut self) {
        let list = self.state.non_steam.games.clone();
        self.state.games.retain(|g| !list.iter().any(|n| n.id == g.app_id.0));
        for n in &list {
            let mut game = Game::new(AppId(n.id), n.name.clone());
            game.install = InstallState::Installed { size_bytes: 0, path: n.folder().display().to_string() };
            // Our own play time is all there is; `merge_installed` adds it on top of zero.
            self.base_play.insert(n.id, (0, 0));
            self.state.games.push(game);
        }
        if !list.is_empty() {
            self.state.games.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
        }
    }

    /// After `merge_installed` reset the install state from Steam's folders: the non-Steam games are "installed".
    pub(crate) fn mark_non_steam_installed(&mut self) {
        for n in &self.state.non_steam.games {
            if let Some(g) = self.state.games.iter_mut().find(|g| g.app_id.0 == n.id) {
                g.install = InstallState::Installed { size_bytes: 0, path: n.folder().display().to_string() };
            }
        }
    }

    pub(crate) fn play_non_steam(&mut self, g: NonSteamGame) {
        let app = AppId(g.id);
        if self.state.running.contains_key(&app) {
            return;
        }
        let env = self.state.game_env.get(app);
        let proton_log = self.state.settings.proton_log;
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let send = move |ev: Event| {
                let _ = tx.blocking_send(ev);
                wake();
            };
            let fail = |send: &dyn Fn(Event), message: String, details: String| {
                send(Event::PlayFailed(PlayProblem {
                    app,
                    name: g.name.clone(),
                    needs_compat_tool: message.contains("compatibility tool"),
                    message,
                    details,
                    log: None,
                }));
            };
            let mut plan = match run::prepare_exe(g.id, &g.exe, &g.args, proton_log, None) {
                Ok(p) => p,
                Err(e) => return fail(&send, format!("{e:#}"), format!("{} ({})\n", g.name, g.exe.display())),
            };
            env.apply(&mut plan, super::game_env::find_gamemoderun());
            let details = run::describe(&plan);
            match super::play::start(&plan, &send) {
                Ok(pid) => send(Event::GameStarted { app, pid, gamescope: false }),
                Err(e) => fail(&send, format!("{e:#}"), format!("{} ({})\n{details}", g.name, app.0)),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kettle-nonsteam-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn crc_and_ids_follow_steams_formula() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let id = shortcut_id("\"/games/a\"", "A");
        assert!(id & ID_BIT != 0);
        assert_eq!(id, shortcut_id("\"/games/a\"", "A"));
        assert_ne!(id, shortcut_id("\"/games/a\"", "B"));
    }

    #[test]
    fn checks_what_the_user_adds() {
        let d = tmp("add");
        let exe = d.join("My Game.sh");
        std::fs::write(&exe, "#!/bin/sh\n").unwrap();
        let g = make_game("", &exe, " -fullscreen ", None).unwrap();
        assert_eq!(g.name, "My Game");
        assert_eq!(g.args, "-fullscreen");
        assert!(g.id & ID_BIT != 0 && !g.imported && !g.is_windows());
        assert_eq!(g.folder(), d);
        assert!(make_game("x", Path::new("relative/x"), "", None).is_err());
        assert!(make_game("x", &d.join("missing"), "", None).is_err());
        assert!(make_game("x", &exe, "a\nb", None).is_err());
        let g = make_game("Nice\u{7}Name", &exe, "", Some(&d.join("no.png"))).unwrap();
        assert_eq!(g.name, "NiceName");
        assert!(g.icon.is_none(), "a missing icon is dropped");
        let win = d.join("Setup.EXE");
        std::fs::write(&win, "MZ").unwrap();
        assert!(make_game("W", &win, "", None).unwrap().is_windows());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn reads_a_desktop_file() {
        let t = "[Desktop Entry]\nName=Cool App\nExec=/opt/cool/run --flag \"two words\" %U\nIcon=/nonexistent.png\n\
                 [Desktop Action x]\nName=Other\nExec=/bin/false\n";
        let e = parse_desktop(t).unwrap();
        assert_eq!(e.name, "Cool App");
        assert_eq!(e.exe, PathBuf::from("/opt/cool/run"));
        assert_eq!(e.args, "--flag \"two words\"");
        assert!(e.icon.is_none());
        assert!(parse_desktop("[Desktop Entry]\nName=NoExec\n").is_none());
        assert!(parse_desktop("Name=x\nExec=/bin/x\n").is_none(), "needs the [Desktop Entry] section");
        // A bare name is searched in PATH.
        let e = parse_desktop("[Desktop Entry]\nName=Sh\nExec=sh -c \"echo 100%%\"\n").unwrap();
        assert!(e.exe.is_absolute());
        assert_eq!(e.args, "-c \"echo 100%\"");
    }

    /// A shortcuts.vdf the way Steam writes it.
    fn vdf_sample() -> Vec<u8> {
        let mut b: Vec<u8> = Vec::new();
        let s = |b: &mut Vec<u8>, k: &str, v: &str| {
            b.push(1);
            b.extend(k.as_bytes());
            b.push(0);
            b.extend(v.as_bytes());
            b.push(0);
        };
        b.extend(b"\0shortcuts\0");
        b.extend(b"\x000\0");
        b.push(2);
        b.extend(b"appid\0");
        b.extend(0x8123_4567u32.to_le_bytes());
        s(&mut b, "AppName", "Some Game");
        s(&mut b, "Exe", "\"/games/some/run\"");
        s(&mut b, "StartDir", "\"/games/some/\"");
        s(&mut b, "LaunchOptions", "-windowed");
        b.extend(b"\0tags\0");
        s(&mut b, "0", "favorite");
        b.push(8); // end of tags
        b.push(8); // end of entry 0
        b.extend(b"\x001\0");
        s(&mut b, "appname", "No Path");
        s(&mut b, "exe", "relative");
        b.push(8);
        b.push(8); // end of shortcuts
        b.push(8); // end of file
        b
    }

    #[test]
    fn reads_steams_shortcuts_file() {
        let games = parse_shortcuts(&vdf_sample());
        assert_eq!(games.len(), 1, "the entry without an absolute program is skipped");
        let g = &games[0];
        assert_eq!((g.id, g.name.as_str()), (0x8123_4567, "Some Game"));
        assert_eq!(g.exe, PathBuf::from("/games/some/run"));
        assert_eq!(g.args, "-windowed");
        assert!(g.imported);
    }

    #[test]
    fn broken_shortcut_files_give_nothing() {
        assert!(parse_shortcuts(b"").is_empty());
        assert!(parse_shortcuts(b"\0shortcuts\0\0").is_empty());
        let mut cut = vdf_sample();
        cut.truncate(cut.len() / 2);
        assert!(parse_shortcuts(&cut).is_empty());
        assert!(parse_shortcuts(&[0x7f; 64]).is_empty());
    }

    #[test]
    fn list_survives_a_round_trip_and_forces_the_id_bit() {
        let f = File {
            games: vec![NonSteamGame {
                id: 5,
                name: "A".into(),
                exe: "/a".into(),
                args: String::new(),
                icon: None,
                imported: false,
            }],
        };
        let back: File = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(back.games, f.games);
        let old: NonSteamGame = serde_json::from_str(r#"{"id": 1, "name": "N", "exe": "/n"}"#).unwrap();
        assert_eq!(old.args, "");
    }

    #[test]
    fn state_finds_games() {
        let g = NonSteamGame {
            id: ID_BIT | 3,
            name: "A".into(),
            exe: "/a".into(),
            args: String::new(),
            icon: None,
            imported: false,
        };
        let s = NonSteamState { games: vec![g] };
        assert!(s.contains(AppId(ID_BIT | 3)));
        assert!(!s.contains(AppId(3)));
    }
}
