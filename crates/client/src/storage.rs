//! Moving an installed game to another library folder (Settings > Storage).
//!
//! The copy goes into a staging folder next to the final place and is renamed when complete, the app
//! manifest is written last, and the old files are deleted only after all of that worked. A failure or a
//! cancel at any point leaves the original untouched and removes the staging folder.

use super::manage::{self, UninstallPlan};
use anyhow::{Context, Result, ensure};
use sr_steam::content::{self, acf};
use sr_steam::local::InstalledApp;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Shown while a move runs.
#[derive(Debug, Clone, PartialEq)]
pub struct MoveState {
    pub app: sr_model::AppId,
    pub name: String,
    pub to: PathBuf,
    pub done: u64,
    pub total: u64,
    /// Set when the move ended: `Ok` or the reason it failed.
    pub result: Option<Result<(), String>>,
}

impl MoveState {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 { 0.0 } else { (self.done as f64 / self.total as f64).clamp(0.0, 1.0) as f32 }
    }
}

/// Everything a move touches, checked before anything is copied.
#[derive(Debug, PartialEq)]
pub struct MovePlan {
    pub appid: u32,
    pub from: UninstallPlan,
    pub to_library: PathBuf,
    /// `<to>/steamapps/common/<installdir>`
    pub to_dir: PathBuf,
    /// Proton prefix of the game in the old library, moved along so saves stay with the game.
    pub compat_from: Option<PathBuf>,
    pub compat_to: PathBuf,
}

/// Work out and check a move of `app` into `to_library`. Reads the disk (manifest, folders).
pub fn plan_move(app: &InstalledApp, to_library: &Path) -> Result<MovePlan> {
    ensure!(to_library.is_absolute(), "the target library must be an absolute path");
    let from = manage::uninstall_plan(app)?;
    ensure!(from.library != to_library, "the game is already in that library");
    let name = from.dir.file_name().context("the install folder has no name")?;
    let to_dir = to_library.join("steamapps/common").join(name);
    ensure!(!to_dir.exists(), "{} already exists", to_dir.display());
    ensure!(!acf::path(to_library, app.appid).exists(), "the target library already has a manifest for this game");
    let compat_from = from.library.join("steamapps/compatdata").join(app.appid.to_string());
    let compat_to = to_library.join("steamapps/compatdata").join(app.appid.to_string());
    ensure!(!compat_to.exists(), "{} already exists", compat_to.display());
    Ok(MovePlan {
        appid: app.appid,
        to_library: to_library.to_path_buf(),
        to_dir,
        compat_from: compat_from.is_dir().then_some(compat_from),
        compat_to,
        from,
    })
}

/// Bytes the move will copy.
pub fn move_size(plan: &MovePlan) -> u64 {
    tree_size(&plan.from.dir) + plan.compat_from.as_deref().map_or(0, tree_size)
}

/// Total size of the regular files below `dir`; symlinks are not followed.
pub fn tree_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => tree_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}

/// Copy `src` to `dst` (which must not exist), keeping symlinks and file modes. `progress` gets the bytes
/// copied so far after every file; `cancel` stops it between files.
pub fn copy_tree(
    src: &Path,
    dst: &Path,
    cancel: &AtomicBool,
    done: &mut u64,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    std::fs::create_dir_all(dst).with_context(|| format!("create {}", dst.display()))?;
    for entry in std::fs::read_dir(src).with_context(|| format!("read {}", src.display()))? {
        let entry = entry?;
        ensure!(!cancel.load(Ordering::Relaxed), "cancelled");
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_tree(&from, &to, cancel, done, progress)?;
        } else if kind.is_symlink() {
            copy_symlink(&from, &to)?;
        } else if kind.is_file() {
            let n = std::fs::copy(&from, &to).with_context(|| format!("copy {}", from.display()))?;
            *done += n;
            progress(*done);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(from: &Path, to: &Path) -> Result<()> {
    let target = std::fs::read_link(from)?;
    std::os::unix::fs::symlink(target, to).with_context(|| format!("symlink {}", to.display()))
}

#[cfg(not(unix))]
fn copy_symlink(from: &Path, _to: &Path) -> Result<()> {
    anyhow::bail!("can't copy the link {} on this system", from.display())
}

/// Run a checked move. Blocking; call from a worker thread.
pub fn run_move(plan: &MovePlan, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<()> {
    let total = move_size(plan);
    let free = super::downloads::free_space(&plan.to_library);
    if let Some(free) = free {
        ensure!(
            content::estimate::with_margin(total) <= free,
            "not enough room in {}: {} needed, {} free",
            plan.to_library.display(),
            human(content::estimate::with_margin(total)),
            human(free)
        );
    }
    let staging = staging_dir(&plan.to_dir);
    let compat_staging = staging_dir(&plan.compat_to);
    let result = copy_all(plan, &staging, &compat_staging, cancel, progress);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
        let _ = std::fs::remove_dir_all(&compat_staging);
        // Undo what already moved into place, so a retry starts clean.
        let _ = std::fs::remove_dir_all(&plan.to_dir);
        let _ = std::fs::remove_dir_all(&plan.compat_to);
        return result;
    }
    // The old copy goes last; its manifest being gone is what makes the game "move" for the official client.
    manage::run_uninstall(&plan.from).context("the game was copied, but the old files could not be removed")?;
    if let Some(compat) = &plan.compat_from {
        let _ = std::fs::remove_dir_all(compat);
    }
    Ok(())
}

fn copy_all(
    plan: &MovePlan,
    staging: &Path,
    compat_staging: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64),
) -> Result<()> {
    let mut done = 0u64;
    let mut report = |n: u64| progress(n);
    copy_tree(&plan.from.dir, staging, cancel, &mut done, &mut report)?;
    if let Some(compat) = &plan.compat_from {
        copy_tree(compat, compat_staging, cancel, &mut done, &mut report)?;
    }
    // Small files that make the game known in the new library.
    let old_info = content::appinfo_path(&plan.from.library, plan.appid);
    if old_info.is_file() {
        let new_info = content::appinfo_path(&plan.to_library, plan.appid);
        if let Some(parent) = new_info.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&old_info, &new_info)?;
    }
    copy_depot_cache(&plan.from.library, &plan.to_library, plan.appid)?;
    ensure!(!cancel.load(Ordering::Relaxed), "cancelled");
    if let Some(parent) = plan.to_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(staging, &plan.to_dir).context("put the copy in place")?;
    if plan.compat_from.is_some() {
        if let Some(parent) = plan.compat_to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(compat_staging, &plan.compat_to).context("put the prefix in place")?;
    }
    // The manifest last: only now does the new library list the game.
    std::fs::create_dir_all(plan.to_library.join("steamapps"))?;
    let manifest_to = acf::path(&plan.to_library, plan.appid);
    std::fs::copy(&plan.from.manifest, &manifest_to).context("copy the game's manifest")?;
    Ok(())
}

/// Manifests of the installed depots (`depotcache/<depot>_<gid>.manifest`), so the next update can be a delta.
fn copy_depot_cache(from: &Path, to: &Path, appid: u32) -> Result<()> {
    let Some(state) = acf::load(from, appid) else { return Ok(()) };
    for (depot, (gid, _)) in &state.depots {
        let name = format!("{depot}_{gid}.manifest");
        let src = from.join("depotcache").join(&name);
        if src.is_file() {
            std::fs::create_dir_all(to.join("depotcache"))?;
            std::fs::copy(&src, to.join("depotcache").join(&name))?;
        }
    }
    Ok(())
}

fn staging_dir(final_dir: &Path) -> PathBuf {
    let name = final_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    final_dir.with_file_name(format!(".kettle-move-{name}"))
}

fn human(b: u64) -> String {
    format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sr-storage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn install(lib: &Path, appid: u32, dirname: &str) -> InstalledApp {
        let dir = lib.join("steamapps/common").join(dirname);
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::write(dir.join("a.bin"), vec![1u8; 1000]).unwrap();
        std::fs::write(dir.join("data/b.bin"), vec![2u8; 500]).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.bin", dir.join("link")).unwrap();
        let state = acf::AppState {
            appid,
            name: "G".into(),
            install_dir: dirname.into(),
            state_flags: 4,
            ..Default::default()
        };
        acf::save(lib, &state).unwrap();
        InstalledApp {
            appid,
            name: "G".into(),
            install_dir: dir,
            size_on_disk: 1500,
            state_flags: 4,
            build_id: 0,
            last_updated: 0,
        }
    }

    #[test]
    fn moves_a_game_with_its_prefix_and_leaves_nothing_behind() {
        let root = scratch("ok");
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(b.join("steamapps")).unwrap();
        let app = install(&a, 9, "Game");
        let prefix = a.join("steamapps/compatdata/9/pfx");
        std::fs::create_dir_all(&prefix).unwrap();
        std::fs::write(prefix.join("save.dat"), b"progress").unwrap();

        let plan = plan_move(&app, &b).unwrap();
        assert_eq!(move_size(&plan), 1508);
        let mut last = 0;
        run_move(&plan, &AtomicBool::new(false), &mut |n| last = n).unwrap();
        assert_eq!(last, 1508);

        assert_eq!(std::fs::read(b.join("steamapps/common/Game/a.bin")).unwrap().len(), 1000);
        assert_eq!(std::fs::read(b.join("steamapps/compatdata/9/pfx/save.dat")).unwrap(), b"progress");
        assert!(acf::load(&b, 9).is_some());
        #[cfg(unix)]
        assert_eq!(std::fs::read_link(b.join("steamapps/common/Game/link")).unwrap(), Path::new("a.bin"));
        assert!(!a.join("steamapps/common/Game").exists());
        assert!(!acf::path(&a, 9).exists());
        assert!(!a.join("steamapps/compatdata/9").exists());
        assert!(!b.join("steamapps/common/.kettle-move-Game").exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cancel_leaves_the_original_and_cleans_up() {
        let root = scratch("cancel");
        let (a, b) = (root.join("a"), root.join("b"));
        std::fs::create_dir_all(b.join("steamapps")).unwrap();
        let app = install(&a, 9, "Game");
        let plan = plan_move(&app, &b).unwrap();
        let err = run_move(&plan, &AtomicBool::new(true), &mut |_| {}).unwrap_err();
        assert!(format!("{err:#}").contains("cancelled"));
        assert!(a.join("steamapps/common/Game/a.bin").exists());
        assert!(acf::path(&a, 9).exists());
        assert!(!b.join("steamapps/common/Game").exists());
        assert!(!b.join("steamapps/common/.kettle-move-Game").exists());
        assert!(!acf::path(&b, 9).exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refuses_bad_targets() {
        let root = scratch("bad");
        let (a, b) = (root.join("a"), root.join("b"));
        let app = install(&a, 9, "Game");
        assert!(plan_move(&app, &a).is_err()); // same library
        assert!(plan_move(&app, Path::new("relative")).is_err());
        std::fs::create_dir_all(b.join("steamapps/common/Game")).unwrap();
        assert!(plan_move(&app, &b).is_err()); // the folder is taken
        let _ = std::fs::remove_dir_all(&root);
    }
}
