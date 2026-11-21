//! App metadata from PICS (Product Info Cache Server): name, install folder,
//! depots with their manifests per branch, build ids and launch entries.

use super::CmLink;
use crate::vdf::{self, Node};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use steam_vent_proto_steam::steammessages_clientserver_appinfo::{
    CMsgClientPICSAccessTokenRequest, CMsgClientPICSAccessTokenResponse, CMsgClientPICSProductInfoRequest,
    CMsgClientPICSProductInfoResponse, cmsg_client_picsproduct_info_request::AppInfo as AppRequest,
};

#[derive(Debug, Clone, Default)]
pub struct DepotInfo {
    pub id: u32,
    /// `config/oslist`, e.g. ["windows"] or ["linux", "macos"]; empty = every OS.
    pub os: Vec<String>,
    /// `config/osarch`: "32", "64" or empty.
    pub arch: String,
    /// `config/language`; empty = language independent.
    pub language: String,
    /// Content comes from this other app (`depotfromapp`).
    pub from_app: Option<u32>,
    pub dlc_app: Option<u32>,
    pub shared_install: bool,
    /// Manifest gid per branch name.
    pub manifests: BTreeMap<String, u64>,
    /// Per branch: (size on disk, download size) as PICS states them next to the manifest id. Missing for
    /// the old one-number manifest format.
    pub sizes: BTreeMap<String, (u64, u64)>,
}

#[derive(Debug, Clone, Default)]
pub struct Branch {
    pub build_id: u64,
    pub time_updated: u64,
    /// The branch is password protected.
    pub password_required: bool,
}

#[derive(Debug, Clone, Default)]
pub struct LaunchEntry {
    pub id: String,
    pub executable: String,
    pub arguments: String,
    pub working_dir: String,
    pub description: String,
    /// `type`: "default", "none", "vr", ...
    pub kind: String,
    pub os: Vec<String>,
    pub arch: String,
    pub beta_key: String,
    pub owns_dlc: String,
}

#[derive(Debug, Clone, Default)]
pub struct AppInfo {
    pub appid: u32,
    pub name: String,
    pub kind: String,
    pub install_dir: String,
    /// `common/oslist` (what the store page says the app supports).
    pub os: Vec<String>,
    pub depots: Vec<DepotInfo>,
    pub branches: BTreeMap<String, Branch>,
    pub launch: Vec<LaunchEntry>,
    /// DLC app ids (`extended/listofdlc` plus the apps depots are tied to).
    pub dlc: Vec<u32>,
    /// The PICS text as received; saved next to the install so the launcher
    /// works without the official client's `appinfo.vdf`.
    pub raw: String,
}

impl AppInfo {
    pub fn build_id(&self, branch: &str) -> u64 {
        self.branches.get(branch).map_or(0, |b| b.build_id)
    }

    pub fn depot(&self, id: u32) -> Option<&DepotInfo> {
        self.depots.iter().find(|d| d.id == id)
    }
}

fn split_list(s: Option<&str>) -> Vec<String> {
    s.unwrap_or("").split(',').map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty()).collect()
}

fn num(node: &Node, key: &str) -> u64 {
    node.get_u64(key).unwrap_or(0)
}

/// Parse the text KeyValues that PICS returns for an app.
pub fn parse(appid: u32, text: &str) -> Result<AppInfo> {
    let doc = vdf::parse(text.trim().trim_matches('\0'));
    let root = doc.get("appinfo").unwrap_or(&doc);
    let mut app = AppInfo { appid, raw: text.to_string(), ..Default::default() };
    if let Some(c) = root.get("common") {
        app.name = c.get_str("name").unwrap_or_default().to_string();
        app.kind = c.get_str("type").unwrap_or_default().to_string();
        app.os = split_list(c.get_str("oslist"));
    }
    app.install_dir = root.path(&["config", "installdir"]).and_then(Node::str).unwrap_or_default().to_string();
    if let Some(d) = root.get("depots") {
        app.depots = d.children().iter().filter_map(|(k, v)| depot(k, v)).collect();
        if let Some(b) = d.get("branches") {
            for (name, v) in b.children() {
                let branch = Branch {
                    build_id: num(v, "buildid"),
                    time_updated: num(v, "timeupdated"),
                    password_required: num(v, "pwdrequired") != 0,
                };
                app.branches.insert(name.clone(), branch);
            }
        }
    }
    let mut dlc: Vec<u32> = root
        .path(&["extended", "listofdlc"])
        .and_then(Node::str)
        .unwrap_or_default()
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect();
    dlc.extend(app.depots.iter().filter_map(|d| d.dlc_app));
    dlc.sort_unstable();
    dlc.dedup();
    app.dlc = dlc;
    if let Some(l) = root.path(&["config", "launch"]) {
        app.launch = l.children().iter().map(|(k, v)| launch(k, v)).collect();
    }
    Ok(app)
}

fn depot(key: &str, v: &Node) -> Option<DepotInfo> {
    let id: u32 = key.parse().ok()?; // skips "branches", "baselanguages", ...
    let cfg = v.get("config");
    let cs = |k: &str| cfg.and_then(|c| c.get_str(k)).unwrap_or_default().to_string();
    let mut manifests = BTreeMap::new();
    let mut sizes = BTreeMap::new();
    for (branch, m) in v.get("manifests").map(Node::children).unwrap_or_default() {
        // Current format: `public { gid "..." size "..." }`; older: `public "gid"`.
        let gid = m.get_str("gid").or_else(|| m.str()).and_then(|s| s.trim().parse::<u64>().ok());
        if let Some(g) = gid.filter(|g| *g != 0) {
            manifests.insert(branch.clone(), g);
            sizes.insert(branch.clone(), (num(m, "size"), num(m, "download")));
        }
    }
    Some(DepotInfo {
        id,
        os: split_list(cfg.and_then(|c| c.get_str("oslist"))),
        arch: cs("osarch"),
        language: cs("language").to_ascii_lowercase(),
        from_app: v.get_u64("depotfromapp").map(|x| x as u32).filter(|x| *x != 0),
        dlc_app: v.get_u64("dlcappid").map(|x| x as u32).filter(|x| *x != 0),
        shared_install: num(v, "sharedinstall") != 0,
        manifests,
        sizes,
    })
}

fn launch(key: &str, v: &Node) -> LaunchEntry {
    let s = |k: &str| v.get_str(k).unwrap_or_default().to_string();
    let cfg = v.get("config");
    let cs = |k: &str| cfg.and_then(|c| c.get_str(k)).unwrap_or_default().to_string();
    LaunchEntry {
        id: key.to_string(),
        executable: s("executable"),
        arguments: s("arguments"),
        working_dir: s("workingdir"),
        description: s("description"),
        kind: s("type"),
        os: split_list(cfg.and_then(|c| c.get_str("oslist"))),
        arch: cs("osarch"),
        beta_key: cs("betakey"),
        owns_dlc: cs("ownsdlc"),
    }
}

/// Ask PICS for the app's metadata. Free/anonymous apps need no token; for
/// others Steam hands one out when the account owns the app.
pub async fn fetch<C: CmLink>(conn: &C, appid: u32) -> Result<AppInfo> {
    let tokens: CMsgClientPICSAccessTokenResponse =
        conn.job(CMsgClientPICSAccessTokenRequest { appids: vec![appid], ..Default::default() }).await?;
    let token = tokens.app_access_tokens.iter().find(|t| t.appid() == appid).map_or(0, |t| t.access_token());
    let req = CMsgClientPICSProductInfoRequest {
        apps: vec![AppRequest {
            appid: Some(appid),
            access_token: Some(token),
            only_public_obsolete: Some(true),
            ..Default::default()
        }],
        meta_data_only: Some(false),
        single_response: Some(true),
        ..Default::default()
    };
    let resp: CMsgClientPICSProductInfoResponse = conn.job(req).await?;
    let entry = resp.apps.iter().find(|a| a.appid() == appid).context("Steam returned no info for this app")?;
    if entry.missing_token() {
        bail!("app {appid}: Steam requires an access token (not owned by this account?)");
    }
    parse(appid, &String::from_utf8_lossy(entry.buffer()))
}

/// Ask PICS for many apps at once (batches of [`BATCH`]). Apps Steam won't tell us about (no access token)
/// are left out; so are answers that don't parse.
pub async fn fetch_many<C: CmLink>(conn: &C, appids: &[u32]) -> Result<Vec<AppInfo>> {
    const BATCH: usize = 40;
    let mut out = Vec::new();
    for ids in appids.chunks(BATCH) {
        let tokens: CMsgClientPICSAccessTokenResponse =
            conn.job(CMsgClientPICSAccessTokenRequest { appids: ids.to_vec(), ..Default::default() }).await?;
        let apps = ids
            .iter()
            .map(|&id| AppRequest {
                appid: Some(id),
                access_token: Some(
                    tokens.app_access_tokens.iter().find(|t| t.appid() == id).map_or(0, |t| t.access_token()),
                ),
                only_public_obsolete: Some(true),
                ..Default::default()
            })
            .collect();
        let req = CMsgClientPICSProductInfoRequest {
            apps,
            meta_data_only: Some(false),
            single_response: Some(true),
            ..Default::default()
        };
        let resp: CMsgClientPICSProductInfoResponse = conn.job(req).await?;
        for entry in resp.apps.iter().filter(|a| !a.missing_token()) {
            if let Ok(info) = parse(entry.appid(), &String::from_utf8_lossy(entry.buffer())) {
                out.push(info);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#""appinfo" { "appid" "7"
      "common" { "name" "Demo" "type" "Tool" "oslist" "windows,linux" }
      "config" { "installdir" "Demo Dir" "launch" {
          "0" { "executable" "demo.exe" "arguments" "-x" "config" { "oslist" "windows" } }
          "1" { "executable" "demo.sh" "type" "default" "config" { "oslist" "linux" "osarch" "64" } } } }
      "depots" { "baselanguages" "english" "8" { "config" { "oslist" "linux" "osarch" "64" }
            "manifests" { "public" { "gid" "123" "size" "9" } "beta" "456" } }
         "9" { "config" { "language" "French" } "dlcappid" "55" "depotfromapp" "3" "sharedinstall" "1" }
         "branches" { "public" { "buildid" "777" "timeupdated" "5" } "x" { "buildid" "8" "pwdrequired" "1" } } } }"#;

    #[test]
    fn parses_appinfo() {
        let a = parse(7, SAMPLE).unwrap();
        assert_eq!((a.name.as_str(), a.install_dir.as_str()), ("Demo", "Demo Dir"));
        assert_eq!(a.os, ["windows", "linux"]);
        assert_eq!(a.depots.len(), 2);
        let d = a.depot(8).unwrap();
        assert_eq!((d.os.clone(), d.arch.as_str()), (vec!["linux".to_string()], "64"));
        assert_eq!(d.manifests["public"], 123);
        assert_eq!(d.manifests["beta"], 456);
        assert_eq!(d.sizes["public"], (9, 0));
        assert_eq!(a.dlc, [55]);
        let d9 = a.depot(9).unwrap();
        assert_eq!(
            (d9.language.as_str(), d9.dlc_app, d9.from_app, d9.shared_install),
            ("french", Some(55), Some(3), true)
        );
        assert_eq!(a.build_id("public"), 777);
        assert!(a.branches["x"].password_required);
        assert_eq!(a.launch.len(), 2);
        assert_eq!(a.launch[1].executable, "demo.sh");
        assert_eq!(a.launch[1].arch, "64");
    }
}
