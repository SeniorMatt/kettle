//! Kettle updates itself: look for a newer release on GitHub, download the AppImage next to the running one and
//! swap it in.
//!
//! Only when Kettle runs as an AppImage (`$APPIMAGE` is set). The check asks the public GitHub releases list
//! (pre-releases included) and sends nothing but a generic `Kettle` User-Agent. A repository that answers 404
//! (private, no token) is treated as "no update", quietly. The download goes to a temporary file in the same
//! folder as the AppImage, is checked (size, ELF header) and then renamed over it, so a failed download never
//! damages the working copy. Nothing is installed without the user's OK.

use super::{Client, Event};
use serde::Deserialize;
use std::cmp::Ordering;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const RELEASES_URL: &str = "https://api.github.com/repos/SeniorMatt/kettle/releases";
/// The only file name we install.
pub const ASSET_NAME: &str = "kettle-x86_64.AppImage";
const DOWNLOAD_PREFIX: &str = "https://github.com/SeniorMatt/kettle/releases/download/";
/// How often the background check runs.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// The release list is small; refuse anything bigger.
const MAX_LIST_BYTES: u64 = 4 << 20;
/// An AppImage smaller than this is not a Kettle build.
const MIN_APPIMAGE_BYTES: u64 = 1 << 20;
const MAX_APPIMAGE_BYTES: u64 = 2 << 30;

// ---- versions -----------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ident {
    Num(u64),
    Text(String),
}

/// A semantic version with pre-release tags: `0.1.0-alpha.5 < 0.1.0-alpha.6 < 0.1.0-beta < 0.1.0`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    pre: Vec<Ident>,
}

impl Version {
    /// Parse `1.2.3`, `v1.2.3-alpha.4` (a leading `v` and `+build` parts are ignored). Fewer than three numbers
    /// count as zeros.
    pub fn parse(text: &str) -> Option<Version> {
        let t = text.trim().trim_start_matches(['v', 'V']);
        let t = t.split('+').next()?;
        let (core, pre) = match t.split_once('-') {
            Some((c, p)) => (c, Some(p)),
            None => (t, None),
        };
        let mut nums = [0u64; 3];
        let mut count = 0;
        for part in core.split('.') {
            if count == 3 {
                return None;
            }
            nums[count] = part.parse().ok()?;
            count += 1;
        }
        if count == 0 {
            return None;
        }
        let pre = match pre {
            None => Vec::new(),
            Some(p) => {
                let ids: Vec<Ident> = p
                    .split('.')
                    .map(|id| match id.parse::<u64>() {
                        Ok(n) => Ident::Num(n),
                        Err(_) => Ident::Text(id.to_string()),
                    })
                    .collect();
                if ids.iter().any(|i| matches!(i, Ident::Text(t) if t.is_empty())) {
                    return None;
                }
                ids
            }
        };
        Some(Version { core: nums, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&other.pre) {
                    let o = match (a, b) {
                        (Ident::Num(x), Ident::Num(y)) => x.cmp(y),
                        (Ident::Num(_), Ident::Text(_)) => Ordering::Less,
                        (Ident::Text(_), Ident::Num(_)) => Ordering::Greater,
                        (Ident::Text(x), Ident::Text(y)) => x.cmp(y),
                    };
                    if o != Ordering::Equal {
                        return o;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Is `candidate` newer than `current`? Unparsable versions are never newer.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    match (Version::parse(current), Version::parse(candidate)) {
        (Some(c), Some(n)) => n > c,
        _ => false,
    }
}

// ---- releases -----------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub prerelease: bool,
    /// The release page (for "what's new").
    pub page: String,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    assets: Vec<RawAsset>,
}

/// The releases from the API's JSON, drafts left out.
pub fn parse_releases(json: &str) -> anyhow::Result<Vec<Release>> {
    let raw: Vec<RawRelease> = serde_json::from_str(json)?;
    Ok(raw
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| Release {
            tag: r.tag_name,
            prerelease: r.prerelease,
            page: r.html_url,
            assets: r
                .assets
                .into_iter()
                .map(|a| Asset { name: a.name, url: a.browser_download_url, size: a.size })
                .collect(),
        })
        .collect())
}

/// The AppImage of a release: exactly [`ASSET_NAME`], from our own repository's download address.
pub fn pick_asset(r: &Release) -> Option<&Asset> {
    r.assets.iter().find(|a| a.name == ASSET_NAME && a.url.starts_with(DOWNLOAD_PREFIX) && a.size > 0)
}

/// What the UI shows about an update that can be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    /// `0.1.0-alpha.6`
    pub version: String,
    pub prerelease: bool,
    /// The release page.
    pub page: String,
    pub size: u64,
    pub(crate) url: String,
}

/// The newest release that is newer than `current` and has an AppImage.
pub fn newest_update(current: &str, releases: &[Release]) -> Option<UpdateInfo> {
    releases
        .iter()
        .filter_map(|r| Some((Version::parse(&r.tag)?, r, pick_asset(r)?)))
        .filter(|(v, _, _)| Version::parse(current).is_some_and(|c| *v > c))
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, r, a)| UpdateInfo {
            version: r.tag.trim_start_matches(['v', 'V']).to_string(),
            prerelease: r.prerelease,
            page: r.page.clone(),
            size: a.size,
            url: a.url.clone(),
        })
}

// ---- state --------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    /// Running as an AppImage and up to date (shown after a manual check).
    UpToDate,
    Available(UpdateInfo),
    Downloading {
        version: String,
        done: u64,
        total: u64,
    },
    /// Installed; the next start runs the new version.
    Ready {
        version: String,
    },
    Failed(String),
}

#[derive(Debug, Clone, Default)]
pub struct KettleUpdateState {
    pub status: UpdateStatus,
    /// Kettle runs as an AppImage, so it can replace itself (`$APPIMAGE` is set).
    pub can_install: bool,
    /// The version that was announced already (so the notice appears once).
    announced: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KettleUpdateCmd {
    /// Look now (the background check runs at start and every 6 hours).
    Check,
    /// The user said OK: download and install the update shown in `Available`.
    Install,
    /// Start the new version and close this one.
    Restart,
    /// Hide the "available" message for this run.
    Dismiss,
    /// "Check for Kettle updates" in Settings.
    SetEnabled(bool),
}

#[derive(Debug)]
pub(crate) enum Ev {
    Tick,
    Checked { manual: bool, result: Result<Option<UpdateInfo>, String> },
    Progress { done: u64, total: u64 },
    Installed(Result<(), String>),
}

/// The running AppImage, when there is one.
pub fn appimage_path() -> Option<PathBuf> {
    std::env::var_os("APPIMAGE").filter(|v| !v.is_empty()).map(PathBuf::from).filter(|p| p.is_absolute())
}

// ---- network and disk ---------------------------------------------------------------------------------------

fn fetch_update(current: &str) -> Result<Option<UpdateInfo>, String> {
    let text = match sr_steam::http_dl::get_text(RELEASES_URL, MAX_LIST_BYTES) {
        Ok(t) => t,
        Err(e) => return Err(format!("{e:#}")),
    };
    let releases = parse_releases(&text).map_err(|e| format!("GitHub sent an unexpected answer: {e}"))?;
    Ok(newest_update(current, &releases))
}

/// Download to `dest` (a temp file next to the AppImage), calling `progress(done, total)`.
fn download(url: &str, expected: u64, dest: &Path, progress: &mut dyn FnMut(u64, u64)) -> anyhow::Result<()> {
    anyhow::ensure!(url.starts_with(DOWNLOAD_PREFIX), "unexpected download address");
    let mut resp = sr_steam::http_dl::agent().get(url).call()?;
    let mut body = resp.body_mut().as_reader();
    let mut out = std::fs::File::create(dest)?;
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    let mut last = std::time::Instant::now();
    loop {
        let n = body.read(&mut buf)?;
        if n == 0 {
            break;
        }
        done += n as u64;
        anyhow::ensure!(done <= MAX_APPIMAGE_BYTES && done <= expected, "the download is larger than announced");
        out.write_all(&buf[..n])?;
        if last.elapsed() >= Duration::from_millis(250) {
            last = std::time::Instant::now();
            progress(done, expected);
        }
    }
    out.flush()?;
    progress(done, expected);
    Ok(())
}

/// Is the file a plausible AppImage of the announced size? (Size, and the ELF header every AppImage starts with.)
pub fn check_download(path: &Path, expected: u64) -> anyhow::Result<()> {
    let len = std::fs::metadata(path)?.len();
    anyhow::ensure!(len == expected, "the download has the wrong size ({len} of {expected} bytes)");
    anyhow::ensure!(len >= MIN_APPIMAGE_BYTES, "the download is too small to be Kettle");
    let mut head = [0u8; 4];
    std::fs::File::open(path)?.read_exact(&mut head)?;
    anyhow::ensure!(&head == b"\x7fELF", "the download is not a program file");
    Ok(())
}

/// Temp file name next to `target`.
pub fn temp_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".update.tmp");
    target.with_file_name(name)
}

fn install(info: &UpdateInfo, target: &Path, progress: &mut dyn FnMut(u64, u64)) -> anyhow::Result<()> {
    let tmp = temp_path(target);
    let result = (|| {
        download(&info.url, info.size, &tmp, progress)?;
        check_download(&tmp, info.size)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        // Same folder, so the rename is atomic and a running AppImage keeps its old (unlinked) file open.
        std::fs::rename(&tmp, target)?;
        anyhow::Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

// ---- the client side ----------------------------------------------------------------------------------------

impl Client {
    /// Start the background check (now, then every 6 hours). Called once from `Client::new`.
    pub(crate) fn start_kettle_update_ticker(&mut self) {
        self.state.kettle_update.can_install = appimage_path().is_some();
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn(async move {
            loop {
                if tx.send(Event::KettleUpdate(Ev::Tick)).await.is_err() {
                    break;
                }
                wake();
                tokio::time::sleep(CHECK_EVERY).await;
            }
        });
    }

    pub(crate) fn kettle_update_cmd(&mut self, cmd: KettleUpdateCmd) {
        match cmd {
            KettleUpdateCmd::Check => self.check_kettle_update(true),
            KettleUpdateCmd::Install => self.install_kettle_update(),
            KettleUpdateCmd::Restart => restart_now(),
            KettleUpdateCmd::Dismiss => {
                if matches!(self.state.kettle_update.status, UpdateStatus::Available(_) | UpdateStatus::Failed(_)) {
                    self.state.kettle_update.status = UpdateStatus::Idle;
                }
            }
            KettleUpdateCmd::SetEnabled(on) => {
                self.state.settings.check_kettle_updates = on;
                self.save_settings();
                if on {
                    self.check_kettle_update(false);
                }
            }
        }
    }

    fn check_kettle_update(&mut self, manual: bool) {
        let ku = &mut self.state.kettle_update;
        if matches!(ku.status, UpdateStatus::Checking | UpdateStatus::Downloading { .. } | UpdateStatus::Ready { .. }) {
            return;
        }
        if !ku.can_install {
            if manual {
                ku.status = UpdateStatus::Failed("Kettle can only update itself when it runs as an AppImage.".into());
            }
            return;
        }
        ku.status = UpdateStatus::Checking;
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let result = fetch_update(env!("CARGO_PKG_VERSION"));
            let _ = tx.blocking_send(Event::KettleUpdate(Ev::Checked { manual, result }));
            wake();
        });
    }

    fn install_kettle_update(&mut self) {
        let UpdateStatus::Available(info) = self.state.kettle_update.status.clone() else { return };
        let Some(target) = appimage_path() else { return };
        self.state.kettle_update.status =
            UpdateStatus::Downloading { version: info.version.clone(), done: 0, total: info.size };
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let send = |ev: Ev| {
                let _ = tx.blocking_send(Event::KettleUpdate(ev));
                wake();
            };
            let result = install(&info, &target, &mut |done, total| send(Ev::Progress { done, total }));
            send(Ev::Installed(result.map_err(|e| format!("{e:#}"))));
        });
    }

    pub(crate) fn apply_kettle_update(&mut self, ev: Ev) {
        match ev {
            Ev::Tick => {
                if self.state.settings.check_kettle_updates {
                    self.check_kettle_update(false);
                }
            }
            Ev::Checked { manual, result } => {
                let ku = &mut self.state.kettle_update;
                ku.status = match result {
                    Ok(Some(info)) => {
                        if ku.announced.as_deref() != Some(info.version.as_str()) {
                            ku.announced = Some(info.version.clone());
                            self.state.notice = Some(format!("Update available: Kettle {}.", info.version));
                        }
                        UpdateStatus::Available(info)
                    }
                    Ok(None) if manual => UpdateStatus::UpToDate,
                    Ok(None) => UpdateStatus::Idle,
                    Err(e) if manual => UpdateStatus::Failed(format!("Couldn't check for updates: {e}")),
                    // A private repository (404) or no network: say nothing.
                    Err(_) => UpdateStatus::Idle,
                };
            }
            Ev::Progress { done, total } => {
                if let UpdateStatus::Downloading { done: d, total: t, .. } = &mut self.state.kettle_update.status {
                    (*d, *t) = (done, total);
                }
            }
            Ev::Installed(result) => {
                let version = match &self.state.kettle_update.status {
                    UpdateStatus::Downloading { version, .. } => version.clone(),
                    _ => String::new(),
                };
                self.state.kettle_update.status = match result {
                    Ok(()) => UpdateStatus::Ready { version },
                    Err(e) => UpdateStatus::Failed(format!("The update failed: {e}")),
                };
            }
        }
    }
}

/// Start the (new) AppImage a moment from now and close this process.
fn restart_now() {
    let Some(path) = appimage_path() else { return };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg("sleep 1; exec \"$0\"").arg(&path);
        cmd.process_group(0).stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        if cmd.spawn().is_ok() {
            std::process::exit(0);
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_with_prerelease_tags() {
        assert!(is_newer("0.1.0-alpha.5", "0.1.0-alpha.6"));
        assert!(is_newer("0.1.0-alpha.5", "v0.1.0-alpha.10"));
        assert!(!is_newer("0.1.0-alpha.6", "0.1.0-alpha.5"));
        assert!(!is_newer("0.1.0-alpha.5", "0.1.0-alpha.5"));
        assert!(is_newer("0.1.0-alpha.9", "0.1.0-beta.1"));
        assert!(is_newer("0.1.0-rc.1", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.1.0-rc.1"));
        assert!(is_newer("0.1.9", "0.2.0"));
        assert!(is_newer("0.9.0", "1.0.0"));
        assert!(is_newer("1.2.3-alpha", "1.2.3-alpha.1"));
        assert!(is_newer("1.0.0-1", "1.0.0-alpha"));
        assert!(!is_newer("0.1.0", "banana"));
        assert!(!is_newer("0.1.0", "0.1.0+build5"));
        assert_eq!(Version::parse("v1.2"), Version::parse("1.2.0"));
        assert!(Version::parse("1.2.3.4").is_none());
        assert!(Version::parse("").is_none());
    }

    fn release(tag: &str, pre: bool, assets: &[(&str, u64)]) -> Release {
        Release {
            tag: tag.into(),
            prerelease: pre,
            page: format!("https://github.com/SeniorMatt/kettle/releases/tag/{tag}"),
            assets: assets
                .iter()
                .map(|(n, s)| Asset { name: (*n).into(), url: format!("{DOWNLOAD_PREFIX}{tag}/{n}"), size: *s })
                .collect(),
        }
    }

    #[test]
    fn picks_the_appimage_only() {
        let r = release("v0.1.0-alpha.6", true, &[("kettle-x86_64.zip", 5), (ASSET_NAME, 99), ("SHA256SUMS", 1)]);
        assert_eq!(pick_asset(&r).map(|a| a.size), Some(99));
        let none = release("v0.1.0-alpha.6", true, &[("kettle-x86_64.zip", 5)]);
        assert!(pick_asset(&none).is_none());
        // A file with the right name from another address is refused.
        let mut evil = release("v0.1.0-alpha.6", true, &[(ASSET_NAME, 99)]);
        evil.assets[0].url = "https://evil.example/kettle-x86_64.AppImage".into();
        assert!(pick_asset(&evil).is_none());
    }

    #[test]
    fn finds_the_newest_update() {
        let list = vec![
            release("v0.1.0-alpha.5", true, &[(ASSET_NAME, 10)]),
            release("v0.1.0-alpha.7", true, &[(ASSET_NAME, 30)]),
            release("v0.1.0-alpha.6", true, &[(ASSET_NAME, 20)]),
            release("v0.1.0-alpha.8", true, &[("notes.txt", 1)]), // no AppImage: skipped
        ];
        let u = newest_update("0.1.0-alpha.5", &list).expect("update");
        assert_eq!((u.version.as_str(), u.size), ("0.1.0-alpha.7", 30));
        assert!(newest_update("0.1.0-alpha.7", &list).is_none());
        assert!(newest_update("0.1.0-alpha.9", &list).is_none());
    }

    #[test]
    fn reads_the_api_answer() {
        let json = r#"[
          {"tag_name":"v0.1.0-alpha.6","draft":false,"prerelease":true,"html_url":"https://github.com/x",
           "assets":[{"name":"kettle-x86_64.AppImage","size":123,
                      "browser_download_url":"https://github.com/SeniorMatt/kettle/releases/download/v0.1.0-alpha.6/kettle-x86_64.AppImage"}]},
          {"tag_name":"v9.9.9","draft":true,"assets":[]}
        ]"#;
        let r = parse_releases(json).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].prerelease);
        assert!(pick_asset(&r[0]).is_some());
        assert!(parse_releases(r#"{"message":"Not Found"}"#).is_err());
    }

    #[test]
    fn download_checks_size_and_header() {
        let dir = std::env::temp_dir().join(format!("kettle-upd-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good");
        let mut data = b"\x7fELF".to_vec();
        data.resize(2 << 20, 0);
        std::fs::write(&good, &data).unwrap();
        assert!(check_download(&good, data.len() as u64).is_ok());
        assert!(check_download(&good, 5).is_err());
        let bad = dir.join("bad");
        let mut html = b"<html".to_vec();
        html.resize(2 << 20, b' ');
        std::fs::write(&bad, &html).unwrap();
        assert!(check_download(&bad, html.len() as u64).is_err());
        let small = dir.join("small");
        std::fs::write(&small, b"\x7fELF").unwrap();
        assert!(check_download(&small, 4).is_err());
        assert_eq!(temp_path(Path::new("/a/kettle.AppImage")), Path::new("/a/kettle.AppImage.update.tmp"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
