//! Our own on-disk data (XDG dirs on Linux, %APPDATA%/%LOCALAPPDATA% on
//! Windows): accounts list and cached library for a warm start. No secrets here.
//!
//! Files are private to the user: mode 0700/0600 on Unix; on Windows the
//! per-user profile folders already have user-only ACLs.

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use std::path::{Path, PathBuf};

pub fn dirs() -> directories::ProjectDirs {
    directories::ProjectDirs::from("", "", "steam-rust").expect("no home directory")
}

pub fn cache_dir() -> PathBuf {
    dirs().cache_dir().to_path_buf()
}

pub fn data_dir() -> PathBuf {
    dirs().data_dir().to_path_buf()
}

/// Create `dir` (and parents) readable only by the current user.
pub fn private_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(dir)?;
    Ok(())
}

/// Write a private (0600) file atomically.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(p) = path.parent() {
        private_dir(p)?;
    }
    let tmp = path.with_extension("tmp");
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn save_json<T: Serialize>(path: &Path, v: &T) -> Result<()> {
    write_private(path, &serde_json::to_vec(v)?)
}

pub fn load_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}
