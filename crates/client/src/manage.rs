//! Managing an installed game: uninstall and "Browse local files".
//!
//! Deleting is the one destructive thing we do, so it is paranoid: the folder must be exactly
//! `<library>/steamapps/common/<installdir>`, must be a real directory (not a symlink), and must match the
//! `installdir` in the game's own `appmanifest`. Anything else is refused. Steam libraries need an explicit
//! confirmation from the UI; our own library does not.

use super::{Client, Event};
use anyhow::{Result, bail, ensure};
use sr_model::AppId;
use sr_steam::content::acf;
use sr_steam::local::InstalledApp;
use std::path::{Component, Path, PathBuf};

/// What an uninstall removes.
#[derive(Debug, PartialEq)]
pub struct UninstallPlan {
    pub library: PathBuf,
    pub dir: PathBuf,
    pub manifest: PathBuf,
}

/// Work out, and check, what to delete for `app`. Reads the manifest from disk.
pub fn uninstall_plan(app: &InstalledApp) -> Result<UninstallPlan> {
    let dir = &app.install_dir;
    ensure!(dir.is_absolute(), "the install folder is not an absolute path");
    ensure!(dir.components().all(|c| !matches!(c, Component::ParentDir | Component::CurDir)), "unsafe install folder");
    let Some(library) = super::downloads::library_of(dir) else {
        bail!("{} is not inside a library's steamapps/common folder", dir.display());
    };
    let Some(name) = dir.file_name().and_then(|n| n.to_str()).filter(|n| !n.is_empty()) else {
        bail!("the install folder has no name");
    };
    let meta = std::fs::symlink_metadata(dir)?;
    ensure!(meta.is_dir(), "{} is not a real folder", dir.display());
    let state = acf::load(&library, app.appid);
    ensure!(
        state.as_ref().is_some_and(|s| s.install_dir == name),
        "the game's manifest does not name {name} as its folder"
    );
    Ok(UninstallPlan { manifest: acf::path(&library, app.appid), library, dir: dir.clone() })
}

/// Delete what the plan names. Blocking.
pub fn run_uninstall(plan: &UninstallPlan) -> Result<()> {
    std::fs::remove_dir_all(&plan.dir)?;
    if let Some(appid) = acf_appid(&plan.manifest) {
        // Launch data we saved at install time; gone with the game.
        let _ = std::fs::remove_file(sr_steam::content::appinfo_path(&plan.library, appid));
    }
    match std::fs::remove_file(&plan.manifest) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// `appmanifest_<id>.acf` -> `<id>`.
fn acf_appid(manifest: &Path) -> Option<u32> {
    manifest.file_name()?.to_str()?.strip_prefix("appmanifest_")?.strip_suffix(".acf")?.parse().ok()
}

impl Client {
    /// Is `library` one we manage (default or the user's chosen download library)?
    pub(crate) fn is_our_library(&self, library: &Path) -> bool {
        self.state.libraries.iter().any(|l| l.ours && l.path == library)
    }

    pub(crate) fn uninstall(&mut self, app: AppId, confirmed: bool) {
        if self.state.running.contains_key(&app) || self.state.downloads.status(app) != super::DownloadStatus::None {
            self.state.notice = Some("Stop the game or its download first.".into());
            return;
        }
        let Some(inst) = self.installed.get(&app.0) else { return };
        let plan = match uninstall_plan(inst) {
            Ok(p) => p,
            Err(e) => {
                self.state.notice = Some(format!("Won't uninstall: {e:#}"));
                return;
            }
        };
        if !confirmed && !self.is_our_library(&plan.library) {
            self.state.notice = Some("Uninstalling from a Steam library needs your confirmation.".into());
            return;
        }
        self.spawn_store(
            move || {
                run_uninstall(&plan)?;
                super::shortcuts::remove(app.0);
                Ok(())
            },
            move |r| Event::Uninstalled { app, result: r },
        );
    }

    pub(crate) fn browse_files(&mut self, app: AppId) {
        let Some(inst) = self.installed.get(&app.0) else { return };
        let dirs: Vec<&Path> = self.installed.values().map(|a| a.install_dir.as_path()).collect();
        let r = super::open::folder(&inst.install_dir, &dirs);
        self.notice_on_err(r);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sr-manage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn app_at(dir: PathBuf) -> InstalledApp {
        InstalledApp {
            appid: 5,
            name: "G".into(),
            install_dir: dir,
            size_on_disk: 1,
            state_flags: 4,
            build_id: 0,
            last_updated: 0,
        }
    }

    fn install(lib: &Path) -> PathBuf {
        let dir = lib.join("steamapps/common/Game");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.bin"), b"x").unwrap();
        let state = acf::AppState {
            appid: 5,
            name: "G".into(),
            install_dir: "Game".into(),
            state_flags: 4,
            ..Default::default()
        };
        acf::save(lib, &state).unwrap();
        dir
    }

    #[test]
    fn removes_folder_and_manifest_only() {
        let lib = scratch("ok");
        let dir = install(&lib);
        std::fs::create_dir_all(lib.join("steamapps/common/Other")).unwrap();
        let plan = uninstall_plan(&app_at(dir.clone())).unwrap();
        run_uninstall(&plan).unwrap();
        assert!(!dir.exists());
        assert!(!acf::path(&lib, 5).exists());
        assert!(lib.join("steamapps/common/Other").exists());
        let _ = std::fs::remove_dir_all(&lib);
    }

    #[test]
    fn refuses_outside_paths_and_mismatched_manifest() {
        let lib = scratch("bad");
        let dir = install(&lib);
        // Not under steamapps/common.
        assert!(uninstall_plan(&app_at(lib.join("steamapps"))).is_err());
        assert!(uninstall_plan(&app_at(PathBuf::from("/"))).is_err());
        assert!(uninstall_plan(&app_at(lib.join("steamapps/common/Game/../Game"))).is_err());
        // Manifest names another folder.
        let other = lib.join("steamapps/common/Other");
        std::fs::create_dir_all(&other).unwrap();
        assert!(uninstall_plan(&app_at(other)).is_err());
        assert!(dir.exists());
        // A symlink is never followed or deleted.
        #[cfg(unix)]
        {
            let link = lib.join("steamapps/common/Link");
            std::os::unix::fs::symlink(&dir, &link).unwrap();
            assert!(uninstall_plan(&app_at(link)).is_err());
        }
        let _ = std::fs::remove_dir_all(&lib);
    }
}
