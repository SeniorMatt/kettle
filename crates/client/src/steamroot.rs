//! A Kettle-owned Steam root so games can find Kettle's own `steamclient.so` (experimental mode "Run games
//! without the Steam client"). See `docs/steamclient-design.md`, section 5.
//!
//! The root is `<data dir>/steamroot/` with `linux64/steamclient.so` (a copy of the library built next to Kettle).
//! Games look for the library at `~/.steam/sdk64/steamclient.so`; we create that link only when nothing of an
//! official Steam install is there. If one is, we leave every link alone and the caller falls back to the
//! official client (which is then started as usual).

use std::path::{Path, PathBuf};

/// What to do with `~/.steam/sdk64/steamclient.so`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkPlan {
    /// Nothing there: create the link.
    Create,
    /// The link is ours already.
    AlreadyOurs,
    /// Something else owns it (an official install): do not touch. Carries the reason.
    Foreign(String),
}

/// Decide about `<home>/.steam/sdk64/steamclient.so` for a library at `ours`.
pub fn plan_link(home: &Path, ours: &Path) -> LinkPlan {
    let steam = home.join(".steam");
    let link = steam.join("sdk64/steamclient.so");
    if let Ok(target) = std::fs::read_link(&link) {
        return if target == ours {
            LinkPlan::AlreadyOurs
        } else {
            LinkPlan::Foreign(format!("{} points to {}", link.display(), target.display()))
        };
    }
    // A link that is a plain file, a directory of the official client, or a `~/.steam/steam` root.
    if std::fs::symlink_metadata(&link).is_ok() {
        return LinkPlan::Foreign(format!("{} exists and is not ours", link.display()));
    }
    if std::fs::symlink_metadata(steam.join("steam")).is_ok() || std::fs::symlink_metadata(steam.join("root")).is_ok() {
        return LinkPlan::Foreign(format!("an official Steam install is set up in {}", steam.display()));
    }
    LinkPlan::Create
}

/// The built library: `libsteamclient.so` or `steamclient.so` next to the running Kettle (or in `../lib`).
pub fn library_source() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.to_path_buf(), dir.join("../lib"), dir.join("lib")]
        .iter()
        .flat_map(|d| [d.join("libsteamclient.so"), d.join("steamclient.so")])
        .find(|p| p.is_file())
}

/// Copy `src` to `<root>/linux64/steamclient.so` (when missing or different) and make the link for games.
/// Returns the root. `Err` carries a sentence for the user; nothing outside our own folders is changed then.
pub fn prepare(root: &Path, src: &Path, home: &Path) -> Result<PathBuf, String> {
    let dest = root.join("linux64/steamclient.so");
    let plan = plan_link(home, &dest);
    if let LinkPlan::Foreign(why) = &plan {
        return Err(format!("An official Steam install is in the way ({why}); Kettle leaves it alone"));
    }
    std::fs::create_dir_all(root.join("linux64")).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let same = match (std::fs::metadata(src), std::fs::metadata(&dest)) {
        (Ok(a), Ok(b)) => a.len() == b.len() && b.modified().ok() >= a.modified().ok(),
        _ => false,
    };
    if !same {
        // Copy to a temp name first: a game that is loading the old library must not see a half-written file.
        let tmp = dest.with_extension("so.new");
        std::fs::copy(src, &tmp).map_err(|e| format!("copying {}: {e}", src.display()))?;
        std::fs::rename(&tmp, &dest).map_err(|e| format!("installing the library: {e}"))?;
    }
    if plan == LinkPlan::Create {
        let sdk = home.join(".steam/sdk64");
        std::fs::create_dir_all(&sdk).map_err(|e| format!("creating {}: {e}", sdk.display()))?;
        std::os::unix::fs::symlink(&dest, sdk.join("steamclient.so")).map_err(|e| format!("linking: {e}"))?;
    }
    Ok(root.to_path_buf())
}

/// Our root inside the data folder.
pub fn default_root() -> PathBuf {
    sr_steam::store::data_dir().join("steamroot")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("kettle-steamroot-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_clean_home_gets_the_link_and_the_copy() {
        let d = tmp("clean");
        let (home, root, src) = (d.join("home"), d.join("root"), d.join("libsteamclient.so"));
        fs::create_dir_all(&home).unwrap();
        fs::write(&src, b"elf").unwrap();
        assert_eq!(prepare(&root, &src, &home).unwrap(), root);
        let link = home.join(".steam/sdk64/steamclient.so");
        assert_eq!(fs::read_link(&link).unwrap(), root.join("linux64/steamclient.so"));
        assert_eq!(fs::read(&link).unwrap(), b"elf");
        // Again: the link is ours, nothing breaks, a changed library is replaced.
        fs::write(&src, b"elf2").unwrap();
        prepare(&root, &src, &home).unwrap();
        assert_eq!(fs::read(&link).unwrap(), b"elf2");
    }

    #[test]
    fn an_official_install_is_never_touched() {
        let d = tmp("official");
        let (home, root, src) = (d.join("home"), d.join("root"), d.join("lib.so"));
        fs::write(&src, b"elf").unwrap();
        // The official client's link.
        fs::create_dir_all(home.join(".steam/sdk64")).unwrap();
        fs::write(home.join(".steam/sdk64/real.so"), b"valve").unwrap();
        std::os::unix::fs::symlink(home.join(".steam/sdk64/real.so"), home.join(".steam/sdk64/steamclient.so"))
            .unwrap();
        let err = prepare(&root, &src, &home).unwrap_err();
        assert!(err.contains("official Steam install"), "{err}");
        assert_eq!(fs::read(home.join(".steam/sdk64/steamclient.so")).unwrap(), b"valve");
        assert!(!root.exists(), "nothing was created");
        // Only a `~/.steam/steam` root, no sdk64 yet: still not ours to redirect.
        let home2 = d.join("home2");
        fs::create_dir_all(home2.join(".steam")).unwrap();
        std::os::unix::fs::symlink(d.join("elsewhere"), home2.join(".steam/steam")).unwrap();
        assert!(matches!(plan_link(&home2, &root.join("linux64/steamclient.so")), LinkPlan::Foreign(_)));
    }
}
