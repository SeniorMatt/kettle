//! Our own on-disk data (XDG dirs): accounts list and cached library for a
//! warm start. No secrets here.

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use std::path::PathBuf;

pub fn dirs() -> directories::ProjectDirs {
    directories::ProjectDirs::from("", "", "steam-rust").expect("no home directory")
}

pub fn cache_dir() -> PathBuf {
    dirs().cache_dir().to_path_buf()
}

pub fn data_dir() -> PathBuf {
    dirs().data_dir().to_path_buf()
}

pub fn save_json<T: Serialize>(path: &PathBuf, v: &T) -> Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec(v)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

pub fn load_json<T: DeserializeOwned>(path: &PathBuf) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}
