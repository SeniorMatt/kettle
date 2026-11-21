//! How big an install will be, before anything is downloaded, from PICS data alone: the depot sizes Steam
//! publishes next to each manifest id. Also lists what the install dialog offers: DLC and languages.

use super::CmLink;
use super::depots::{self, Os, Target};
use super::product::{self, AppInfo};
use anyhow::{Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DlcInfo {
    pub appid: u32,
    pub name: String,
    /// Disk size of the depots that belong to this DLC (0 when PICS doesn't say).
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallInfo {
    pub app: u32,
    pub name: String,
    pub os: Os,
    /// Size on disk of the base game's depots, for the chosen OS and language.
    pub disk_bytes: u64,
    /// Bytes to download (compressed), when PICS states them.
    pub download_bytes: u64,
    pub dlc: Vec<DlcInfo>,
    /// Languages the app has depots for, "english" first. Empty = one language only.
    pub languages: Vec<String>,
}

/// Free space an install of `bytes` should find: the size plus 3 % and at least 64 MiB for the manifest
/// files, temporary data and the file system's own overhead.
pub fn with_margin(bytes: u64) -> u64 {
    bytes.saturating_add((bytes / 33).max(64 << 20))
}

/// Disk and download size of the depots picked by `target`, optionally only those tied to `only_dlc`.
pub fn sizes(app: &AppInfo, target: &Target, only_dlc: Option<u32>) -> (u64, u64) {
    depots::select(app, target)
        .into_iter()
        .filter(|d| match only_dlc {
            Some(id) => d.dlc_app == Some(id),
            None => d.dlc_app.is_none(),
        })
        .filter_map(|d| d.sizes.get(target.branch))
        .fold((0, 0), |(a, b), (size, dl)| (a + size, b + dl))
}

/// Languages of the app's depots, "english" first, others sorted.
pub fn languages(app: &AppInfo, os: Os) -> Vec<String> {
    let mut langs: Vec<String> = app
        .depots
        .iter()
        .filter(|d| d.os.is_empty() || d.os.iter().any(|o| o == os.as_str()))
        .filter(|d| !d.language.is_empty())
        .map(|d| d.language.clone())
        .collect();
    langs.sort();
    langs.dedup();
    if let Some(i) = langs.iter().position(|l| l == "english") {
        let e = langs.remove(i);
        langs.insert(0, e);
    }
    langs
}

/// Build the dialog's data from an app's PICS record. `dlc_names` gives the DLC names PICS returned.
pub fn from_app(app: &AppInfo, os: Option<Os>, language: &str, dlc_names: &[AppInfo]) -> InstallInfo {
    let os = os.unwrap_or_else(|| Os::default_for(app));
    let target = Target { os, language, branch: "public", dlc: &[] };
    let (disk_bytes, download_bytes) = sizes(app, &target, None);
    let dlc = app
        .dlc
        .iter()
        .map(|&id| {
            let one = [id];
            let t = Target { os, language, branch: "public", dlc: &one };
            let name = dlc_names.iter().find(|a| a.appid == id).map(|a| a.name.clone()).unwrap_or_default();
            DlcInfo { appid: id, name, bytes: sizes(app, &t, Some(id)).0 }
        })
        .collect();
    InstallInfo {
        app: app.appid,
        name: app.name.clone(),
        os,
        disk_bytes,
        download_bytes,
        dlc,
        languages: languages(app, os),
    }
}

/// Fetch what the install dialog needs over an open connection.
pub async fn estimate<C: CmLink>(conn: &C, appid: u32, os: Option<Os>, language: &str) -> Result<InstallInfo> {
    let app = product::fetch(conn, appid).await.context("app info")?;
    // DLC names are a nicety: a failure only leaves them blank.
    let ids: Vec<u32> = app.dlc.iter().copied().take(60).collect();
    let dlc_apps = if ids.is_empty() { Vec::new() } else { product::fetch_many(conn, &ids).await.unwrap_or_default() };
    Ok(from_app(&app, os, language, &dlc_apps))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::product::DepotInfo;

    fn depot(id: u32, lang: &str, dlc: Option<u32>, size: u64, dl: u64) -> DepotInfo {
        let mut d = DepotInfo { id, language: lang.into(), dlc_app: dlc, ..Default::default() };
        d.manifests.insert("public".into(), 1);
        d.sizes.insert("public".into(), (size, dl));
        d
    }

    fn app() -> AppInfo {
        let mut a = AppInfo { appid: 5, name: "Game".into(), os: vec!["linux".into()], ..Default::default() };
        a.depots = vec![
            depot(1, "", None, 1000, 400),
            depot(2, "english", None, 50, 20),
            depot(3, "german", None, 60, 25),
            depot(4, "", Some(77), 300, 100),
        ];
        a.dlc = vec![77];
        a
    }

    #[test]
    fn sums_base_and_dlc_separately() {
        let info = from_app(&app(), Some(Os::Linux), "english", &[]);
        assert_eq!((info.disk_bytes, info.download_bytes), (1050, 420));
        assert_eq!(info.dlc, [DlcInfo { appid: 77, name: String::new(), bytes: 300 }]);
        let de = from_app(&app(), Some(Os::Linux), "german", &[]);
        assert_eq!(de.disk_bytes, 1060);
    }

    #[test]
    fn languages_list_english_first() {
        assert_eq!(languages(&app(), Os::Linux), ["english", "german"]);
    }

    #[test]
    fn margin_is_at_least_64_mib() {
        assert_eq!(with_margin(0), 64 << 20);
        assert_eq!(with_margin(33 << 30), (33u64 << 30) + (1 << 30));
    }
}
