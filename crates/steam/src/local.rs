//! Read-only access to an existing official Steam installation, so games it
//! already installed show up as installed (no re-download). Never writes.

use crate::vdf::{self, Node};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Candidate Steam roots, most common first: Debian/Ubuntu package, native,
/// Flatpak, Snap. Deduplicated by canonical path (the Debian layout has
/// `~/.steam/steam` and `~/.steam/root` symlinks to the same place).
pub fn steam_roots() -> Vec<PathBuf> {
    let mut seen = Vec::new();
    for c in root_candidates() {
        if !c.join("steamapps").is_dir() && !c.join("config/libraryfolders.vdf").is_file() {
            continue;
        }
        if let Ok(canon) = c.canonicalize()
            && !seen.contains(&canon)
        {
            seen.push(canon);
        }
    }
    seen
}

/// Where the official client may be installed on Linux: the Debian/Ubuntu
/// package, a native install, the usual symlinks, Flatpak and Snap.
#[cfg(not(windows))]
fn root_candidates() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return vec![] };
    vec![
        home.join(".steam/debian-installation"),
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".steam/root"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
        home.join("snap/steam/common/.local/share/Steam"),
    ]
}

/// On Windows the client records its folder in the registry; fall back to
/// the default install location.
#[cfg(windows)]
fn root_candidates() -> Vec<PathBuf> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let mut out = Vec::new();
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam")
        && let Ok(path) = key.get_value::<String, _>("SteamPath")
    {
        out.push(PathBuf::from(path));
    }
    out.push(PathBuf::from("C:\\Program Files (x86)\\Steam"));
    out
}

/// Steam library folders listed by `steamapps/libraryfolders.vdf` of a root.
pub fn library_folders(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    if let Ok(src) = std::fs::read_to_string(root.join("steamapps/libraryfolders.vdf")) {
        let doc = vdf::parse(&src);
        if let Some(lf) = doc.get("libraryfolders") {
            for (_, entry) in lf.children() {
                if let Some(p) = entry.get_str("path") {
                    let p = PathBuf::from(p);
                    if !out.contains(&p) && p.join("steamapps").is_dir() {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct InstalledApp {
    pub appid: u32,
    pub name: String,
    pub install_dir: PathBuf,
    pub size_on_disk: u64,
    /// AppState StateFlags: 4 = fully installed; 2 = update required; 1024 = updating ...
    pub state_flags: u32,
    pub build_id: u64,
    pub last_updated: u64,
}

impl InstalledApp {
    pub fn fully_installed(&self) -> bool {
        self.state_flags & 4 != 0
    }
}

/// Apps that are runtime/tools, not games (hidden from the library).
pub fn is_tool_app(appid: u32, name: &str) -> bool {
    const TOOLS: &[u32] =
        &[228980, 1070560, 1391110, 1628350, 1493710, 1580130, 1887720, 1826330, 2180100, 2348590, 2805730, 3658110];
    TOOLS.contains(&appid)
        || name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name.starts_with("Steamworks Common Redistributables")
}

/// All apps installed under all known libraries of all Steam roots.
pub fn installed_apps() -> HashMap<u32, InstalledApp> {
    let mut out = HashMap::new();
    for root in steam_roots().into_iter().chain([crate::content::default_library()]) {
        for lib in library_folders(&root) {
            let apps = lib.join("steamapps");
            let Ok(rd) = std::fs::read_dir(&apps) else { continue };
            for e in rd.flatten() {
                let fname = e.file_name();
                let f = fname.to_string_lossy();
                if !(f.starts_with("appmanifest_") && f.ends_with(".acf")) {
                    continue;
                }
                let Ok(src) = std::fs::read_to_string(e.path()) else { continue };
                if let Some(app) = parse_manifest(&vdf::parse(&src), &apps) {
                    out.insert(app.appid, app);
                }
            }
        }
    }
    out
}

fn parse_manifest(doc: &Node, steamapps: &Path) -> Option<InstalledApp> {
    let s = doc.get("AppState")?;
    Some(InstalledApp {
        appid: s.get_u64("appid")? as u32,
        name: s.get_str("name").unwrap_or_default().to_string(),
        install_dir: steamapps.join("common").join(s.get_str("installdir").unwrap_or_default()),
        size_on_disk: s.get_u64("SizeOnDisk").unwrap_or(0),
        state_flags: s.get_u64("StateFlags").unwrap_or(0) as u32,
        build_id: s.get_u64("buildid").unwrap_or(0),
        last_updated: s.get_u64("LastUpdated").unwrap_or(0),
    })
}

/// Cached library art for an app from the official client's cache, if any.
/// Layouts: `librarycache/<appid>/<file>`, `librarycache/<appid>/<hash>/<file>`
/// (2024+ client) and the older flat `librarycache/<appid>_<file>`.
pub fn cached_art(root: &Path, appid: u32, file: &str) -> Option<PathBuf> {
    let cache = root.join("appcache/librarycache");
    let dir = cache.join(appid.to_string());
    let direct = dir.join(file);
    if direct.is_file() {
        return Some(direct);
    }
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let nested = e.path().join(file);
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    let flat = cache.join(format!("{appid}_{file}"));
    flat.is_file().then_some(flat)
}
