//! The `appmanifest_<appid>.acf` file Steam keeps for each installed app, so
//! the official client recognises a folder we filled when it's added as a
//! library. Also reads back what we installed last time.

use crate::vdf;
use anyhow::Result;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// StateFlags: fully installed / update running.
pub const STATE_INSTALLED: u32 = 4;
pub const STATE_UPDATING: u32 = 1026;

#[derive(Debug, Clone, Default)]
pub struct AppState {
    pub appid: u32,
    pub name: String,
    pub install_dir: String,
    pub state_flags: u32,
    pub last_updated: u64,
    pub size_on_disk: u64,
    pub build_id: u64,
    pub last_owner: u64,
    /// depot -> (manifest gid, installed size)
    pub depots: BTreeMap<u32, (u64, u64)>,
}

pub fn path(library: &Path, appid: u32) -> PathBuf {
    library.join("steamapps").join(format!("appmanifest_{appid}.acf"))
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Render in the exact shape Steam writes (tab-separated, same field order).
pub fn render(a: &AppState) -> String {
    let mut s = String::from("\"AppState\"\n{\n");
    let mut kv = |k: &str, v: String| s.push_str(&format!("\t\"{k}\"\t\t\"{}\"\n", esc(&v)));
    kv("appid", a.appid.to_string());
    kv("universe", "1".into());
    kv("name", a.name.clone());
    kv("StateFlags", a.state_flags.to_string());
    kv("installdir", a.install_dir.clone());
    kv("lastupdated", a.last_updated.to_string());
    kv("LastPlayed", "0".into());
    kv("SizeOnDisk", a.size_on_disk.to_string());
    kv("StagingSize", "0".into());
    kv("buildid", a.build_id.to_string());
    kv("LastOwner", a.last_owner.to_string());
    kv("DownloadType", "1".into());
    kv("UpdateResult", "0".into());
    kv("BytesToDownload", "0".into());
    kv("BytesDownloaded", "0".into());
    kv("BytesToStage", "0".into());
    kv("BytesStaged", "0".into());
    kv("TargetBuildID", a.build_id.to_string());
    kv("AutoUpdateBehavior", "0".into());
    kv("AllowOtherDownloadsWhileRunning", "0".into());
    s.push_str("\t\"InstalledDepots\"\n\t{\n");
    for (depot, (gid, size)) in &a.depots {
        s.push_str(&format!(
            "\t\t\"{depot}\"\n\t\t{{\n\t\t\t\"manifest\"\t\t\"{gid}\"\n\t\t\t\"size\"\t\t\"{size}\"\n\t\t}}\n"
        ));
    }
    s.push_str("\t}\n\t\"UserConfig\"\n\t{\n\t}\n\t\"MountedConfig\"\n\t{\n\t}\n}\n");
    s
}

pub fn parse(text: &str) -> Option<AppState> {
    let doc = vdf::parse(text);
    let s = doc.get("AppState")?;
    let mut depots = BTreeMap::new();
    for (id, d) in s.get("InstalledDepots").map(vdf::Node::children).unwrap_or_default() {
        if let (Ok(id), Some(gid)) = (id.parse(), d.get_u64("manifest")) {
            depots.insert(id, (gid, d.get_u64("size").unwrap_or(0)));
        }
    }
    Some(AppState {
        appid: s.get_u64("appid")? as u32,
        name: s.get_str("name").unwrap_or_default().to_string(),
        install_dir: s.get_str("installdir").unwrap_or_default().to_string(),
        state_flags: s.get_u64("StateFlags").unwrap_or(0) as u32,
        last_updated: s.get_u64("lastupdated").unwrap_or(0),
        size_on_disk: s.get_u64("SizeOnDisk").unwrap_or(0),
        build_id: s.get_u64("buildid").unwrap_or(0),
        last_owner: s.get_u64("LastOwner").unwrap_or(0),
        depots,
    })
}

pub fn load(library: &Path, appid: u32) -> Option<AppState> {
    parse(&std::fs::read_to_string(path(library, appid)).ok()?)
}

/// Write via a temp file and rename, so a crash never leaves a half file.
pub fn save(library: &Path, a: &AppState) -> Result<()> {
    let p = path(library, a.appid);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = p.with_extension("acf.tmp");
    std::fs::write(&tmp, render(a))?;
    std::fs::rename(tmp, p)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_output_parses_back() {
        let mut a = AppState {
            appid: 1007,
            name: "Steamworks \"SDK\" Redist".into(),
            install_dir: "Steamworks SDK Redist".into(),
            state_flags: STATE_INSTALLED,
            last_updated: 1_700_000_000,
            size_on_disk: 123,
            build_id: 42,
            last_owner: 0,
            ..Default::default()
        };
        a.depots.insert(1006, (99, 123));
        let text = render(&a);
        let b = parse(&text).unwrap();
        assert_eq!((b.appid, b.name.as_str(), b.state_flags, b.build_id), (1007, a.name.as_str(), 4, 42));
        assert_eq!(b.depots[&1006], (99, 123));
        // Our own scanner must accept it too.
        let doc = vdf::parse(&text);
        assert_eq!(doc.path(&["AppState", "installdir"]).and_then(vdf::Node::str), Some("Steamworks SDK Redist"));
    }
}
