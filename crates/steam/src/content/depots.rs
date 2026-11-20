//! Which depots of an app to install on a given OS, architecture and language.

use super::product::{AppInfo, DepotInfo};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    MacOs,
}

impl Os {
    pub fn as_str(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Windows => "windows",
            Os::MacOs => "macos",
        }
    }

    /// Linux when the app lists it, otherwise Windows (what Proton runs).
    pub fn default_for(app: &AppInfo) -> Os {
        if app.os.iter().any(|o| o == "linux") || app.depots.iter().any(|d| d.os.iter().any(|o| o == "linux")) {
            Os::Linux
        } else {
            Os::Windows
        }
    }
}

impl FromStr for Os {
    type Err = String;
    fn from_str(s: &str) -> Result<Os, String> {
        match s.to_ascii_lowercase().as_str() {
            "linux" => Ok(Os::Linux),
            "windows" | "win" => Ok(Os::Windows),
            "macos" | "mac" | "osx" => Ok(Os::MacOs),
            other => Err(format!("unknown OS {other:?} (use linux, windows or macos)")),
        }
    }
}

pub struct Target<'a> {
    pub os: Os,
    pub language: &'a str,
    pub branch: &'a str,
    /// DLC app ids whose depots should be installed too.
    pub dlc: &'a [u32],
}

/// Depots to install, in app order. 64-bit depots win over 32-bit ones for the same OS.
/// A depot is kept even without a manifest of its own when `depotfromapp` points
/// elsewhere; the caller resolves that.
pub fn select<'a>(app: &'a AppInfo, t: &Target) -> Vec<&'a DepotInfo> {
    let for_os = |d: &&DepotInfo| d.os.is_empty() || d.os.iter().any(|o| o == t.os.as_str());
    let candidates: Vec<&DepotInfo> = app
        .depots
        .iter()
        .filter(for_os)
        .filter(|d| d.language.is_empty() || d.language == t.language.to_ascii_lowercase())
        .filter(|d| d.dlc_app.is_none_or(|a| t.dlc.contains(&a)))
        .filter(|d| d.manifests.contains_key(t.branch) || d.from_app.is_some())
        .collect();
    let has_64 = candidates.iter().any(|d| d.arch == "64");
    candidates.into_iter().filter(|d| !(has_64 && d.arch == "32")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn depot(id: u32, os: &[&str], arch: &str, lang: &str) -> DepotInfo {
        DepotInfo {
            id,
            os: os.iter().map(|s| s.to_string()).collect(),
            arch: arch.into(),
            language: lang.into(),
            manifests: BTreeMap::from([("public".to_string(), 1)]),
            ..Default::default()
        }
    }

    #[test]
    fn picks_os_arch_language() {
        let mut app = AppInfo { os: vec!["windows".into(), "linux".into()], ..Default::default() };
        app.depots = vec![
            depot(1, &[], "", ""),
            depot(2, &["windows"], "", ""),
            depot(3, &["linux"], "32", ""),
            depot(4, &["linux"], "64", ""),
            depot(5, &[], "", "french"),
            depot(6, &[], "", "english"),
        ];
        let t = |os| Target { os, language: "English", branch: "public", dlc: &[] };
        let ids = |os| select(&app, &t(os)).iter().map(|d| d.id).collect::<Vec<_>>();
        assert_eq!(Os::default_for(&app), Os::Linux);
        assert_eq!(ids(Os::Linux), [1, 4, 6]);
        assert_eq!(ids(Os::Windows), [1, 2, 6]);
    }

    #[test]
    fn skips_dlc_and_branchless() {
        let mut app = AppInfo::default();
        let mut d = depot(1, &[], "", "");
        d.dlc_app = Some(9);
        let mut e = depot(2, &[], "", "");
        e.manifests.clear();
        app.depots = vec![d, e];
        let mut t = Target { os: Os::Linux, language: "english", branch: "public", dlc: &[] };
        assert!(select(&app, &t).is_empty());
        t.dlc = &[9];
        assert_eq!(select(&app, &t).len(), 1);
        assert_eq!(Os::default_for(&app), Os::Windows);
    }
}
