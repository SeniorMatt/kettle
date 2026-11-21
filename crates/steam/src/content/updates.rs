//! Finding out whether an installed game has an update: compare what the app manifest (`appmanifest_*.acf`)
//! recorded with what PICS says now. Works for games installed by the official client too, because it only
//! reads their manifests.

use super::CmLink;
use super::acf::AppState;
use super::product::{self, AppInfo};
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

/// The branch an installed game follows: its beta key when PICS knows that branch, else "public".
pub fn branch_of<'a>(installed: &'a AppState, app: &AppInfo) -> &'a str {
    if !installed.beta_key.is_empty() && app.branches.contains_key(&installed.beta_key) {
        &installed.beta_key
    } else {
        "public"
    }
}

/// Does `installed` lag behind `app` (the PICS data)? True when the build id differs, or when a depot we have
/// is at another manifest than the branch now offers. A recorded build id of 0 (unknown) never counts on its
/// own, and depots that belong to other apps (not in this app's list) are ignored.
pub fn update_available(installed: &AppState, app: &AppInfo) -> bool {
    let branch = branch_of(installed, app);
    let latest = app.build_id(branch);
    if latest != 0 && installed.build_id != 0 && installed.build_id != latest {
        return true;
    }
    installed.depots.iter().any(|(depot, (have, _))| {
        app.depot(*depot).and_then(|d| d.manifests.get(branch)).is_some_and(|now| now != have)
    })
}

/// Which of `apps` (appid, library root) have an update. Reads each app's manifest from its library, asks
/// PICS in batches, returns the app ids that are behind. Apps without a readable manifest, in the middle of
/// an install or unknown to PICS are left out.
pub async fn find_updates<C: CmLink>(conn: &C, apps: &[(u32, std::path::PathBuf)]) -> Result<Vec<u32>> {
    let states: HashMap<u32, AppState> = apps
        .iter()
        .filter_map(|(id, lib)| load(lib, *id).map(|s| (*id, s)))
        .filter(|(_, s)| s.state_flags & super::acf::STATE_INSTALLED != 0)
        .collect();
    let ids: Vec<u32> = states.keys().copied().collect();
    let infos = product::fetch_many(conn, &ids).await?;
    let mut out: Vec<u32> = infos
        .iter()
        .filter(|i| states.get(&i.appid).is_some_and(|s| update_available(s, i)))
        .map(|i| i.appid)
        .collect();
    out.sort_unstable();
    Ok(out)
}

fn load(library: &Path, appid: u32) -> Option<AppState> {
    super::acf::load(library, appid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::product::{Branch, DepotInfo};
    use std::collections::BTreeMap;

    fn app(build: u64, gid: u64) -> AppInfo {
        let mut a = AppInfo { appid: 7, ..Default::default() };
        a.branches.insert("public".into(), Branch { build_id: build, ..Default::default() });
        a.branches.insert("beta".into(), Branch { build_id: build + 10, ..Default::default() });
        let mut d = DepotInfo { id: 8, ..Default::default() };
        d.manifests.insert("public".into(), gid);
        d.manifests.insert("beta".into(), gid + 1);
        a.depots.push(d);
        a
    }

    fn state(build: u64, gid: u64) -> AppState {
        AppState { appid: 7, build_id: build, depots: BTreeMap::from([(8, (gid, 100))]), ..Default::default() }
    }

    #[test]
    fn same_build_and_manifest_is_current() {
        assert!(!update_available(&state(100, 5), &app(100, 5)));
    }

    #[test]
    fn newer_build_is_an_update() {
        assert!(update_available(&state(100, 5), &app(101, 5)));
    }

    #[test]
    fn manifest_change_alone_is_an_update() {
        assert!(update_available(&state(100, 5), &app(100, 6)));
    }

    #[test]
    fn unknown_installed_build_does_not_count_by_itself() {
        assert!(!update_available(&state(0, 5), &app(100, 5)));
    }

    #[test]
    fn depots_of_other_apps_are_ignored() {
        let mut s = state(100, 5);
        s.depots.insert(999, (1, 1));
        assert!(!update_available(&s, &app(100, 5)));
    }

    #[test]
    fn beta_branch_is_followed() {
        let mut s = state(110, 6);
        s.beta_key = "beta".into();
        // Public moved on to 100 -> not an update for someone on beta 110 with its manifest 6.
        assert!(!update_available(&s, &app(100, 5)));
        assert!(update_available(&s, &app(101, 5)));
        // A beta key PICS no longer lists falls back to public.
        s.beta_key = "gone".into();
        assert_eq!(branch_of(&s, &app(100, 5)), "public");
    }

    #[test]
    fn detects_an_update_from_real_looking_files() {
        // An appmanifest as the official client writes it, and the PICS text of the same app.
        let acf_text = r#""AppState" { "appid" "7" "name" "Demo" "StateFlags" "4" "installdir" "Demo"
            "buildid" "1000" "InstalledDepots" { "8" { "manifest" "123" "size" "9" } } "UserConfig" { "language" "english" } }"#;
        let pics = |build: u32, gid: u32| {
            format!(
                r#""appinfo" {{ "common" {{ "name" "Demo" }} "depots" {{ "8" {{ "manifests" {{ "public" {{ "gid" "{gid}" "size" "9" }} }} }}
                "branches" {{ "public" {{ "buildid" "{build}" }} }} }} }}"#
            )
        };
        let installed = crate::content::acf::parse(acf_text).unwrap();
        assert_eq!(installed.build_id, 1000);
        let same = product::parse(7, &pics(1000, 123)).unwrap();
        let newer = product::parse(7, &pics(1001, 124)).unwrap();
        assert!(!update_available(&installed, &same));
        assert!(update_available(&installed, &newer));
    }
}
