//! User settings (default download library, worker count) and the list of libraries a game can go to.
//!
//! Settings live in a small JSON in our data dir. Nothing secret is stored here.

use super::{Client, Command};
use serde::{Deserialize, Serialize};
use sr_steam::{content, local, store};
use std::path::{Path, PathBuf};

pub const MIN_WORKERS: usize = 1;
pub const MAX_WORKERS: usize = 32;
const DEFAULT_WORKERS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// Where new games go by default; `None` = our own library folder.
    #[serde(default)]
    pub library: Option<PathBuf>,
    /// Chunk downloads in flight.
    #[serde(default = "default_workers")]
    pub workers: usize,
    /// Start the official Steam client in the background for games that need it.
    #[serde(default = "default_true")]
    pub start_steam: bool,
    /// Ask Proton to write its log (`PROTON_LOG=1`) for Windows games.
    #[serde(default)]
    pub proton_log: bool,
    /// The Shift+Tab in-game overlay (the shortcut is registered only while a game we started runs).
    #[serde(default = "default_true")]
    pub overlay: bool,
    /// Animations, scale, start page, tray.
    #[serde(default)]
    pub interface: crate::InterfaceSettings,
    /// The performance overlay (MangoHud): preset, items, position, FPS limit.
    #[serde(default)]
    pub hud: crate::HudSettings,
    /// Run games in Gamescope (off by default) and its options.
    #[serde(default)]
    pub gamescope: crate::GamescopeSettings,
    /// Updates, speed limit, schedule, added library folders.
    #[serde(default)]
    pub downloads: crate::DownloadSettings,
    /// Which desktop notifications to show.
    #[serde(default)]
    pub notifications: crate::NotifySettings,
    /// Keep the sign-in token in the keyring so the next start needs no QR code.
    #[serde(default = "default_true")]
    pub keep_signed_in: bool,
    /// Steam Cloud: download saves before a game starts and upload them after it (a game can opt out).
    #[serde(default = "default_true")]
    pub cloud_sync: bool,
}

fn default_true() -> bool {
    true
}

fn default_workers() -> usize {
    DEFAULT_WORKERS
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            library: None,
            workers: DEFAULT_WORKERS,
            start_steam: true,
            proton_log: false,
            overlay: true,
            interface: Default::default(),
            hud: Default::default(),
            gamescope: Default::default(),
            downloads: Default::default(),
            notifications: Default::default(),
            keep_signed_in: true,
            cloud_sync: true,
        }
    }
}

impl Settings {
    pub(crate) fn path() -> PathBuf {
        store::data_dir().join("settings.json")
    }

    pub fn load() -> Settings {
        let mut s: Settings = store::load_json(&Settings::path()).unwrap_or_default();
        s.workers = s.workers.clamp(MIN_WORKERS, MAX_WORKERS);
        s
    }

    /// The library a download goes to unless the user picks another.
    pub fn default_library(&self) -> PathBuf {
        self.library.clone().unwrap_or_else(content::default_library)
    }
}

/// A place a game can be installed into.
#[derive(Debug, Clone, PartialEq)]
pub struct LibraryChoice {
    pub path: PathBuf,
    pub label: String,
    /// Our own folder (default or chosen): uninstalling there needs no extra confirmation.
    pub ours: bool,
}

/// Our libraries first, then every folder the official client lists.
pub fn library_choices(settings: &Settings) -> Vec<LibraryChoice> {
    let mut out: Vec<LibraryChoice> = Vec::new();
    let mut push = |path: PathBuf, label: &str, ours: bool| {
        if !out.iter().any(|l| l.path == path) {
            out.push(LibraryChoice { path, label: label.to_string(), ours });
        }
    };
    push(settings.default_library(), "Kettle library", true);
    push(content::default_library(), "Kettle library", true);
    for p in &settings.downloads.libraries {
        push(p.clone(), "Kettle library", true);
    }
    for root in local::steam_roots() {
        for lib in local::library_folders(&root) {
            push(lib, "Steam library", false);
        }
    }
    out
}

impl Client {
    pub(crate) fn set_library(&mut self, library: Option<PathBuf>) {
        if let Some(p) = &library
            && !valid_library(p)
        {
            self.state.notice = Some("Pick an absolute folder path for the library.".into());
            return;
        }
        self.state.settings.library = library;
        self.settings_changed();
    }

    pub(crate) fn set_workers(&mut self, workers: usize) {
        self.state.settings.workers = workers.clamp(MIN_WORKERS, MAX_WORKERS);
        self.save_settings();
    }

    pub(crate) fn set_start_steam(&mut self, on: bool) {
        self.state.settings.start_steam = on;
        self.save_settings();
    }

    pub(crate) fn set_proton_log(&mut self, on: bool) {
        self.state.settings.proton_log = on;
        self.save_settings();
    }

    pub(crate) fn set_overlay(&mut self, on: bool) {
        self.state.settings.overlay = on;
        self.save_settings();
    }

    pub(crate) fn set_interface(&mut self, i: crate::InterfaceSettings) {
        self.state.settings.interface = i.clamped();
        self.save_settings();
    }

    pub(crate) fn save_settings(&mut self) {
        if let Err(e) = store::save_json(&Settings::path(), &self.state.settings) {
            self.state.notice = Some(format!("Couldn't save the settings: {e:#}"));
        }
    }

    pub(crate) fn settings_changed(&mut self) {
        self.save_settings();
        self.state.libraries = library_choices(&self.state.settings);
        // A newly chosen library may already hold games.
        self.spawn_local_scan();
    }

    /// Extra library folders to look for installed games in.
    pub(crate) fn extra_libraries(&self) -> Vec<PathBuf> {
        self.state.settings.library.iter().chain(&self.state.settings.downloads.libraries).cloned().collect()
    }
}

fn valid_library(p: &Path) -> bool {
    p.is_absolute() && p.parent().is_some()
}

/// Commands that only touch settings, so `Client::send` stays a short list.
pub(crate) fn handle(client: &mut Client, cmd: Command) {
    match cmd {
        Command::SetLibrary(p) => client.set_library(p),
        Command::SetWorkers(n) => client.set_workers(n),
        Command::SetStartSteam(on) => client.set_start_steam(on),
        Command::SetProtonLog(on) => client.set_proton_log(on),
        Command::SetOverlay(on) => client.set_overlay(on),
        Command::SetInterface(i) => client.set_interface(i),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_clamping() {
        let s = Settings::default();
        assert_eq!(s.workers, 8);
        assert!(s.start_steam);
        assert!(s.overlay);
        let s: Settings = serde_json::from_str(r#"{"overlay": false}"#).unwrap();
        assert!(!s.overlay);
        let s: Settings = serde_json::from_str(r#"{"start_steam": false}"#).unwrap();
        assert!(!s.start_steam);
        let s = Settings::default();
        assert_eq!(s.default_library(), content::default_library());
        let s: Settings = serde_json::from_str(r#"{"workers": 500}"#).unwrap();
        assert_eq!(s.workers, 500); // clamped on load/set, not on parse
        assert!(!valid_library(Path::new("relative/dir")));
        assert!(!valid_library(Path::new("/")));
        assert!(valid_library(Path::new("/games/lib")));
    }

    #[test]
    fn an_old_settings_file_still_loads() {
        // What settings.json looked like before downloads, notifications and the account options existed.
        let old = r#"{"library": "/games/lib", "workers": 4, "start_steam": false, "overlay": true}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.library, Some(PathBuf::from("/games/lib")));
        assert_eq!((s.workers, s.start_steam, s.overlay), (4, false, true));
        assert_eq!(s.downloads, crate::DownloadSettings::default());
        assert_eq!(s.notifications, crate::NotifySettings::default());
        assert!(s.keep_signed_in);
        assert!(s.downloads.auto_update);
        assert_eq!(s.downloads.limit_kbps, 0);
        // Gamescope arrived later: old files get it off, with every option at its default.
        assert_eq!(s.gamescope, crate::GamescopeSettings::default());
        assert!(!s.gamescope.enabled);
        // An empty file and a partly filled new section work too.
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(s.workers, 8);
        let s: Settings = serde_json::from_str(r#"{"downloads": {"limit_kbps": 500}}"#).unwrap();
        assert_eq!(s.downloads.limit_kbps, 500);
        assert!(s.downloads.auto_update);
        // And the new settings survive a round trip.
        let mut s = Settings::default();
        s.downloads.schedule_enabled = true;
        s.downloads.libraries.push("/more".into());
        s.notifications.friend_online = true;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn added_libraries_are_offered_as_ours() {
        let mut s = Settings::default();
        s.downloads.libraries.push("/extra/lib".into());
        let choices = library_choices(&s);
        assert!(choices.iter().any(|l| l.path == Path::new("/extra/lib") && l.ours));
    }
}
