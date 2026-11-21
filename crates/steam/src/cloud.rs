//! Steam Cloud, the pure part: where a game keeps its saves, which files exist locally, what was synced last
//! time and what has to happen now. No network here; `cloud_sync` talks to Steam and runs the plan.
//!
//! Where saves live comes from the app's `ufs` section of appinfo:
//!
//! * `savefiles`: `root` (a place such as `WinMyDocuments`, `gameinstall`, `LinuxXdgDataHome`), a `path` below it,
//!   a file `pattern`, `recursive` and the `platforms` it applies to.
//! * `rootoverrides`: "on this OS use that other root (and extra path) instead". Valve uses them to say where a
//!   Windows save is on Linux.
//!
//! A game that runs through Proton sees Windows, so its Windows roots are folders inside the prefix
//! (`compatdata/<appid>/pfx/drive_c/users/steamuser/...`). A native game sees Linux. Games that use the
//! `ISteamRemoteStorage` API keep their files in `userdata/<accountid>/<appid>/remote` of the Steam folder; in the
//! cloud those files have no `%Root%` prefix.
//!
//! A file is named in the cloud as `%Root%path/file` (for example `%WinMyDocuments%My Games/Foo/slot1.sav`), whatever
//! OS made it, so the same save is found from every machine. NOTE: this naming is taken from how Steam's own
//! client behaves; it has not been checked against a real account here. Names that match no known place are
//! never touched.

use crate::appinfo::Kv;
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// `platforms_to_sync` bits.
pub const PLATFORM_WINDOWS: u32 = 1;
pub const PLATFORM_MACOS: u32 = 2;
pub const PLATFORM_LINUX: u32 = 4;
pub const PLATFORM_ALL: u32 = u32::MAX;

/// Files bigger than this are left alone (Steam's own limit per file is about 100 MB).
pub const MAX_FILE_BYTES: u64 = 100 * 1024 * 1024;
/// Folder depth limit when scanning.
const MAX_DEPTH: usize = 12;

/// The OS the game sees: Windows for Proton / Wine, Linux for a native game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Linux,
    Windows,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Linux => "Linux",
            Target::Windows => "Windows",
        }
    }

    pub fn platform_bit(self) -> u32 {
        match self {
            Target::Linux => PLATFORM_LINUX,
            Target::Windows => PLATFORM_WINDOWS,
        }
    }
}

// ---- ufs -----------------------------------------------------------------------------------------------------

/// One `ufs/savefiles/<n>` entry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SaveFiles {
    pub root: String,
    pub path: String,
    /// Patterns (`*` and `?`); `siblings` adds more.
    pub patterns: Vec<String>,
    pub recursive: bool,
    /// Lower-case platform names (`windows`, `linux`, `macos`, `all`); empty means all.
    pub platforms: Vec<String>,
}

/// One `ufs/rootoverrides/<n>` entry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RootOverride {
    pub root: String,
    pub os: String,
    /// `oscompare` was `!=`.
    pub not_equal: bool,
    pub use_instead: String,
    pub add_path: String,
    pub transforms: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ufs {
    pub save_files: Vec<SaveFiles>,
    pub overrides: Vec<RootOverride>,
}

fn truthy(kv: Option<&Kv>) -> bool {
    kv.and_then(Kv::as_string).is_some_and(|s| matches!(s.trim(), "1" | "true" | "True"))
}

impl Ufs {
    /// From the `ufs` node of appinfo. Entries without a root are dropped.
    pub fn from_kv(ufs: &Kv) -> Ufs {
        let save_files = ufs
            .get("savefiles")
            .map(|s| {
                s.children()
                    .iter()
                    .filter_map(|(_, e)| {
                        let root = e.get_string("root")?;
                        let mut patterns =
                            vec![e.get_string("pattern").filter(|p| !p.is_empty()).unwrap_or("*".into())];
                        if let Some(sib) = e.get_string("siblings") {
                            patterns.extend(sib.split(';').map(str::trim).filter(|p| !p.is_empty()).map(String::from));
                        }
                        let platforms = match e.get("platforms") {
                            Some(Kv::Map(kids)) => kids.iter().filter_map(|(_, v)| v.as_string()).collect(),
                            Some(v) => v.as_string().into_iter().collect(),
                            None => vec![],
                        };
                        Some(SaveFiles {
                            root,
                            path: e.get_string("path").unwrap_or_default(),
                            patterns,
                            recursive: truthy(e.get("recursive")),
                            platforms: platforms.iter().map(|p| p.trim().to_ascii_lowercase()).collect(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let overrides = ufs
            .get("rootoverrides")
            .map(|s| {
                s.children()
                    .iter()
                    .filter_map(|(_, e)| {
                        let transforms = e
                            .get("pathtransforms")
                            .map(|t| {
                                t.children()
                                    .iter()
                                    .filter_map(|(_, x)| Some((x.get_string("find")?, x.get_string("replace")?)))
                                    .collect()
                            })
                            .unwrap_or_default();
                        Some(RootOverride {
                            root: e.get_string("root")?,
                            os: e.get_string("os")?,
                            not_equal: e.get_string("oscompare").is_some_and(|c| c.trim() == "!="),
                            use_instead: e.get_string("useinstead")?,
                            add_path: e.get_string("addpath").unwrap_or_default(),
                            transforms,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ufs { save_files, overrides }
    }

    pub fn is_empty(&self) -> bool {
        self.save_files.is_empty()
    }
}

fn applies(platforms: &[String], target: Target) -> bool {
    platforms.is_empty() || platforms.iter().any(|p| p == "all" || p.eq_ignore_ascii_case(target.name()))
    // appinfo writes the macOS name as "MacOS" or "OSX"; neither is ours, so nothing else to test.
}

// ---- roots ---------------------------------------------------------------------------------------------------

/// Where the roots are, for one launch.
#[derive(Debug, Clone, PartialEq)]
pub struct Env {
    pub target: Target,
    pub install_dir: PathBuf,
    pub home: PathBuf,
    pub xdg_data: PathBuf,
    pub xdg_config: PathBuf,
    /// Proton: `<prefix>/drive_c/users/steamuser`. `None` for a native game.
    pub win_user: Option<PathBuf>,
    /// Proton: `<prefix>/drive_c`.
    pub drive_c: Option<PathBuf>,
    pub steam_id: u64,
}

impl Env {
    /// For the real machine. `prefix` is the Wine prefix (`.../pfx`) when the game runs through Proton / Wine.
    pub fn for_launch(install_dir: &Path, prefix: Option<&Path>, steam_id: u64) -> Env {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let xdg = |var: &str, rel: &str| match std::env::var_os(var).map(PathBuf::from).filter(|p| p.is_absolute()) {
            Some(p) => p,
            None => home.join(rel),
        };
        Env {
            target: if prefix.is_some() { Target::Windows } else { Target::Linux },
            install_dir: install_dir.to_path_buf(),
            xdg_data: xdg("XDG_DATA_HOME", ".local/share"),
            xdg_config: xdg("XDG_CONFIG_HOME", ".config"),
            win_user: prefix.map(|p| p.join("drive_c/users/steamuser")),
            drive_c: prefix.map(|p| p.join("drive_c")),
            home,
            steam_id,
        }
    }

    pub fn account_id(&self) -> u32 {
        (self.steam_id & 0xFFFF_FFFF) as u32
    }

    /// The folder a root name stands for, or `None` when it has no meaning here.
    pub fn root_dir(&self, root: &str) -> Option<PathBuf> {
        let user = |rel: &str| self.win_user.as_ref().map(|u| u.join(rel));
        match root.to_ascii_lowercase().as_str() {
            "gameinstall" => Some(self.install_dir.clone()),
            "winmydocuments" => user("Documents"),
            "winappdatalocal" => user("AppData/Local"),
            "winappdatalocallow" => user("AppData/LocalLow"),
            "winappdataroaming" => user("AppData/Roaming"),
            "winsavedgames" => user("Saved Games"),
            "windesktop" => user("Desktop"),
            "winpictures" => user("Pictures"),
            "winmusic" => user("Music"),
            "winvideos" => user("Videos"),
            "winprogramdata" => self.drive_c.as_ref().map(|c| c.join("ProgramData")),
            "linuxhome" => Some(self.home.clone()),
            "linuxxdgdatahome" => Some(self.xdg_data.clone()),
            "linuxxdgconfighome" => Some(self.xdg_config.clone()),
            _ => None,
        }
        .filter(|p| !p.as_os_str().is_empty())
    }
}

/// The spelling Steam uses inside `%...%` for a root name.
pub fn canonical_root(root: &str) -> String {
    const KNOWN: [&str; 14] = [
        "GameInstall",
        "WinMyDocuments",
        "WinAppDataLocal",
        "WinAppDataLocalLow",
        "WinAppDataRoaming",
        "WinSavedGames",
        "WinDesktop",
        "WinPictures",
        "WinMusic",
        "WinVideos",
        "WinProgramData",
        "LinuxHome",
        "LinuxXdgDataHome",
        "LinuxXdgConfigHome",
    ];
    KNOWN.iter().find(|k| k.eq_ignore_ascii_case(root)).map_or_else(|| root.to_string(), |k| (*k).to_string())
}

/// `a\b//c/` -> `a/b/c`
fn clean_rel(p: &str) -> String {
    p.replace('\\', "/").split('/').filter(|s| !s.is_empty() && *s != ".").collect::<Vec<_>>().join("/")
}

fn substitute(path: &str, env: &Env) -> String {
    path.replace("{64BitSteamID}", &env.steam_id.to_string())
        .replace("{Steam3AccountID}", &env.account_id().to_string())
}

/// One place to look for saves and how it is named in the cloud.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapping {
    /// Cloud name prefix: `%Root%some/path/` (ends with `/` unless the path is empty).
    pub token: String,
    /// The folder on this machine.
    pub dir: PathBuf,
    pub patterns: Vec<String>,
    pub recursive: bool,
}

/// The places to scan for `ufs` on this machine, plus a note for each entry that was skipped.
pub fn mappings(ufs: &Ufs, env: &Env) -> (Vec<Mapping>, Vec<String>) {
    let (mut out, mut notes) = (Vec::new(), Vec::new());
    for sf in &ufs.save_files {
        if !applies(&sf.platforms, env.target) {
            continue;
        }
        let orig_path = clean_rel(&substitute(&sf.path, env));
        let (mut root, mut local_path) = (sf.root.clone(), orig_path.clone());
        let ov = ufs.overrides.iter().find(|o| {
            o.root.eq_ignore_ascii_case(&sf.root) && o.os.eq_ignore_ascii_case(env.target.name()) != o.not_equal
        });
        if let Some(o) = ov {
            root = o.use_instead.clone();
            let mut p = orig_path.clone();
            for (find, replace) in &o.transforms {
                if !find.is_empty() {
                    p = p.replace(find.as_str(), replace);
                }
            }
            local_path = clean_rel(&format!("{}/{}", o.add_path, p));
        }
        let Some(base) = env.root_dir(&root) else {
            notes.push(format!(
                "skipped {}: the place \"{root}\" does not exist for a {} game",
                sf.root,
                env.target.name()
            ));
            continue;
        };
        let token = format!(
            "%{}%{}",
            canonical_root(&sf.root),
            if orig_path.is_empty() { String::new() } else { format!("{orig_path}/") }
        );
        let dir = if local_path.is_empty() { base } else { base.join(&local_path) };
        if !out.iter().any(|m: &Mapping| m.token == token && m.dir == dir) {
            out.push(Mapping { token, dir, patterns: sf.patterns.clone(), recursive: sf.recursive });
        }
    }
    (out, notes)
}

/// `*` and `?` match, case-insensitive.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    fn go(p: &[char], n: &[char]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some(('*', rest)) => (0..=n.len()).any(|i| go(rest, &n[i..])),
            Some(('?', rest)) => !n.is_empty() && go(rest, &n[1..]),
            Some((c, rest)) => n.first().is_some_and(|x| x.eq_ignore_ascii_case(c)) && go(rest, &n[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    go(&p, &n)
}

/// A relative cloud path that stays inside its folder.
fn safe_rel(rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.contains('\0') || rel.contains('\\') {
        return None;
    }
    let p = Path::new(rel);
    p.components().all(|c| matches!(c, Component::Normal(_))).then(|| p.to_path_buf())
}

/// Where a cloud file goes on this machine: `None` when no known place holds it.
pub fn local_path(name: &str, maps: &[Mapping], remote: Option<&Path>) -> Option<PathBuf> {
    if name.starts_with('%') {
        let m = maps
            .iter()
            .filter(|m| {
                name.len() > m.token.len()
                    && name.get(..m.token.len()).is_some_and(|h| h.eq_ignore_ascii_case(&m.token))
            })
            .filter(|m| {
                let rest = &name[m.token.len()..];
                let file = rest.rsplit('/').next().unwrap_or(rest);
                m.patterns.iter().any(|p| glob_match(p, file)) && (m.recursive || !rest.contains('/'))
            })
            .max_by_key(|m| m.token.len())?;
        Some(m.dir.join(safe_rel(&name[m.token.len()..])?))
    } else {
        Some(remote?.join(safe_rel(name)?))
    }
}

// ---- local scan ----------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct LocalFile {
    /// Cloud name.
    pub name: String,
    pub path: PathBuf,
    /// SHA-1, lower-case hex.
    pub sha: String,
    /// Modification time, seconds since 1970.
    pub time: u64,
    pub size: u64,
}

pub fn sha1_hex(bytes: &[u8]) -> String {
    hex(&Sha1::digest(bytes))
}

pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

pub fn unix_secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn walk(dir: &Path, rel: &str, depth: usize, recursive: bool, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let Ok(ft) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        let rel_name = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        if ft.is_file() {
            out.push((rel_name, e.path()));
        } else if ft.is_dir() && recursive && depth < MAX_DEPTH {
            walk(&e.path(), &rel_name, depth + 1, true, out);
        }
    }
}

fn describe_file(name: String, path: PathBuf, skipped: &mut Vec<String>) -> Option<LocalFile> {
    let meta = std::fs::metadata(&path).ok()?;
    if meta.len() > MAX_FILE_BYTES {
        skipped.push(format!("left out {name}: larger than {} MB", MAX_FILE_BYTES / 1024 / 1024));
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    Some(LocalFile {
        name,
        sha: sha1_hex(&bytes),
        time: meta.modified().map(unix_secs).unwrap_or(0),
        size: meta.len(),
        path,
    })
}

/// Every save file on this machine, by cloud name. The first place that holds a name wins.
pub fn scan(maps: &[Mapping], remote: Option<&Path>) -> (Vec<LocalFile>, Vec<String>) {
    let (mut files, mut notes) = (BTreeMap::new(), Vec::new());
    for m in maps {
        let mut found = Vec::new();
        walk(&m.dir, "", 0, m.recursive, &mut found);
        for (rel, path) in found {
            let file = rel.rsplit('/').next().unwrap_or(&rel);
            if !m.patterns.iter().any(|p| glob_match(p, file)) {
                continue;
            }
            let name = format!("{}{rel}", m.token);
            if !files.contains_key(&name)
                && let Some(f) = describe_file(name.clone(), path, &mut notes)
            {
                files.insert(name, f);
            }
        }
    }
    if let Some(dir) = remote {
        let mut found = Vec::new();
        walk(dir, "", 0, true, &mut found);
        for (rel, path) in found {
            if !rel.starts_with('%')
                && !files.contains_key(&rel)
                && let Some(f) = describe_file(rel.clone(), path, &mut notes)
            {
                files.insert(rel, f);
            }
        }
    }
    (files.into_values().collect(), notes)
}

// ---- sync state and decisions --------------------------------------------------------------------------------

/// What both sides agreed on at the end of the last sync.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FileState {
    pub sha: String,
    pub time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SyncState {
    /// The cloud's change number when we last looked (informational; every sync reads the full list).
    #[serde(default)]
    pub change: u64,
    #[serde(default)]
    pub files: BTreeMap<String, FileState>,
}

impl SyncState {
    pub fn path(appid: u32) -> PathBuf {
        crate::store::data_dir().join("cloud").join(format!("{appid}.json"))
    }

    pub fn load(appid: u32) -> SyncState {
        crate::store::load_json(&SyncState::path(appid)).unwrap_or_default()
    }

    pub fn save(&self, appid: u32) -> anyhow::Result<()> {
        crate::store::save_json(&SyncState::path(appid), self)
    }

    pub fn record(&mut self, name: &str, sha: &str, time: u64) {
        self.files.insert(name.to_string(), FileState { sha: sha.to_string(), time });
    }
}

/// A file as the cloud lists it.
#[derive(Debug, Clone, PartialEq)]
pub struct CloudFile {
    pub name: String,
    /// SHA-1, lower-case hex.
    pub sha: String,
    pub time: u64,
    pub size: u32,
    pub deleted: bool,
    pub platforms: u32,
}

impl CloudFile {
    /// Files that are not meant for this OS are ignored in both directions.
    pub fn wanted_on(&self, target: Target) -> bool {
        self.platforms == 0 || self.platforms & target.platform_bit() != 0
    }
}

/// A file changed on both sides since the last sync.
#[derive(Debug, Clone, PartialEq)]
pub struct Conflict {
    pub name: String,
    pub local_time: u64,
    pub cloud_time: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum Resolution {
    KeepLocal,
    KeepCloud,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Plan {
    /// Cloud is newer: fetch.
    pub downloads: Vec<String>,
    /// Local is newer (or new): send.
    pub uploads: Vec<String>,
    /// Both sides have the same content: only the state needs updating.
    pub in_sync: Vec<String>,
    pub conflicts: Vec<Conflict>,
    /// Uploads that come from "keep local" (sent before launch, because the user chose them).
    pub chosen_uploads: Vec<String>,
    pub notes: Vec<String>,
}

impl Plan {
    pub fn is_noop(&self) -> bool {
        self.downloads.is_empty() && self.uploads.is_empty() && self.conflicts.is_empty()
    }

    /// The newest local and cloud times among the conflicting files.
    pub fn conflict_times(&self) -> (u64, u64) {
        (
            self.conflicts.iter().map(|c| c.local_time).max().unwrap_or(0),
            self.conflicts.iter().map(|c| c.cloud_time).max().unwrap_or(0),
        )
    }

    /// Settle every conflict in favour of one side.
    pub fn resolve(&mut self, r: Resolution) {
        for c in std::mem::take(&mut self.conflicts) {
            match r {
                Resolution::KeepLocal => {
                    self.chosen_uploads.push(c.name.clone());
                    self.uploads.push(c.name);
                }
                Resolution::KeepCloud => self.downloads.push(c.name),
            }
        }
    }
}

/// Compare local files, the cloud list and the last agreed state.
///
/// * same content on both sides: nothing to do;
/// * only the cloud differs from the last agreed state: download; only the local file: upload;
/// * both differ from it (or there is no earlier state and the contents differ): conflict;
/// * a file missing locally is fetched again (we cannot tell "deleted by the user" from "folder gone"), and a file
///   the cloud lost stays where it is. Nothing is ever deleted, locally or in the cloud.
pub fn plan(base: &SyncState, local: &[LocalFile], cloud: &[CloudFile]) -> Plan {
    let l: BTreeMap<&str, &LocalFile> = local.iter().map(|f| (f.name.as_str(), f)).collect();
    let c: BTreeMap<&str, &CloudFile> = cloud.iter().filter(|f| !f.deleted).map(|f| (f.name.as_str(), f)).collect();
    let names: BTreeSet<&str> = l.keys().chain(c.keys()).copied().collect();
    let mut p = Plan::default();
    for name in names {
        let b = base.files.get(name);
        match (l.get(name), c.get(name)) {
            (Some(lf), Some(cf)) if lf.sha == cf.sha => p.in_sync.push(name.to_string()),
            (Some(lf), Some(cf)) => {
                let local_changed = b.is_none_or(|b| b.sha != lf.sha);
                let cloud_changed = b.is_none_or(|b| b.sha != cf.sha);
                match (local_changed, cloud_changed) {
                    (true, true) => {
                        p.conflicts.push(Conflict { name: name.to_string(), local_time: lf.time, cloud_time: cf.time })
                    }
                    (false, _) => p.downloads.push(name.to_string()),
                    (true, false) => p.uploads.push(name.to_string()),
                }
            }
            (Some(lf), None) => match b {
                Some(b) if b.sha == lf.sha => p.notes.push(format!("{name}: gone from the cloud, kept locally")),
                _ => p.uploads.push(name.to_string()),
            },
            (None, Some(_)) => p.downloads.push(name.to_string()),
            (None, None) => {}
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vdf;

    fn ufs_from(text: &str) -> Ufs {
        let node = vdf::parse(text);
        Ufs::from_kv(&Kv::from_vdf(node.children()[0].1.get("ufs").expect("ufs")))
    }

    const SAMPLE: &str = r#""appinfo" { "ufs" {
        "quota" "1000000"
        "savefiles" {
            "0" { "root" "WinMyDocuments" "path" "My Games/Foo" "pattern" "*.sav" "recursive" "1"
                  "platforms" { "1" "Windows" } }
            "1" { "root" "gameinstall" "path" "profile" "pattern" "*.dat" "siblings" "*.cfg;x?.bin"
                  "platforms" { "1" "All" } }
            "2" { "root" "WinAppDataLocal" "path" "Bar/{Steam3AccountID}" "pattern" "*" }
        }
        "rootoverrides" {
            "0" { "root" "WinAppDataLocal" "os" "Linux" "oscompare" "=" "useinstead" "LinuxXdgDataHome"
                  "addpath" "bar-game" "pathtransforms" { "0" { "find" "Bar/" "replace" "saves/" } } }
        }
    } }"#;

    fn env(target: Target) -> Env {
        let proton = target == Target::Windows;
        Env {
            target,
            install_dir: PathBuf::from("/lib/steamapps/common/Foo"),
            home: PathBuf::from("/home/u"),
            xdg_data: PathBuf::from("/home/u/.local/share"),
            xdg_config: PathBuf::from("/home/u/.config"),
            win_user: proton.then(|| PathBuf::from("/lib/steamapps/compatdata/5/pfx/drive_c/users/steamuser")),
            drive_c: proton.then(|| PathBuf::from("/lib/steamapps/compatdata/5/pfx/drive_c")),
            steam_id: 0x0110_0001_0000_0000 + 1234,
        }
    }

    #[test]
    fn parses_ufs() {
        let u = ufs_from(SAMPLE);
        assert_eq!(u.save_files.len(), 3);
        assert!(u.save_files[0].recursive && !u.save_files[1].recursive);
        assert_eq!(u.save_files[1].patterns, ["*.dat", "*.cfg", "x?.bin"]);
        assert_eq!(u.save_files[0].platforms, ["windows"]);
        assert_eq!(u.overrides.len(), 1);
        assert_eq!(u.overrides[0].transforms, [("Bar/".to_string(), "saves/".to_string())]);
        assert!(Ufs::from_kv(&Kv::Map(vec![])).is_empty());
    }

    #[test]
    fn proton_roots_are_inside_the_prefix() {
        let (m, notes) = mappings(&ufs_from(SAMPLE), &env(Target::Windows));
        assert!(notes.is_empty());
        let user = "/lib/steamapps/compatdata/5/pfx/drive_c/users/steamuser";
        assert_eq!(m[0].token, "%WinMyDocuments%My Games/Foo/");
        assert_eq!(m[0].dir, PathBuf::from(format!("{user}/Documents/My Games/Foo")));
        assert!(m[0].recursive);
        assert_eq!(m[1].token, "%GameInstall%profile/");
        assert_eq!(m[1].dir, PathBuf::from("/lib/steamapps/common/Foo/profile"));
        // Windows game: no override, the account id is filled in.
        assert_eq!(m[2].dir, PathBuf::from(format!("{user}/AppData/Local/Bar/1234")));
        assert_eq!(m[2].token, "%WinAppDataLocal%Bar/1234/");
    }

    #[test]
    fn native_roots_use_overrides_and_skip_windows_entries() {
        let (m, notes) = mappings(&ufs_from(SAMPLE), &env(Target::Linux));
        assert!(notes.is_empty());
        // entry 0 is Windows only; entry 1 is for all; entry 2 is overridden to the XDG data dir.
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].token, "%GameInstall%profile/");
        // The cloud name keeps the original place, the folder is the override's (with the path transformed).
        assert_eq!(m[1].token, "%WinAppDataLocal%Bar/1234/");
        assert_eq!(m[1].dir, PathBuf::from("/home/u/.local/share/bar-game/saves/1234"));
    }

    #[test]
    fn unknown_or_unmapped_roots_are_noted() {
        let u = ufs_from(r#""x" { "ufs" { "savefiles" { "0" { "root" "MacHome" "path" "p" "pattern" "*" } } } }"#);
        let (m, notes) = mappings(&u, &env(Target::Linux));
        assert!(m.is_empty());
        assert_eq!(notes.len(), 1);
        // A Windows root has no folder for a native game.
        let u =
            ufs_from(r#""x" { "ufs" { "savefiles" { "0" { "root" "WinMyDocuments" "path" "p" "pattern" "*" } } } }"#);
        assert!(mappings(&u, &env(Target::Linux)).0.is_empty());
    }

    #[test]
    fn cloud_names_map_back_safely() {
        let (m, _) = mappings(&ufs_from(SAMPLE), &env(Target::Windows));
        let p = local_path("%WinMyDocuments%My Games/Foo/sub/a.sav", &m, None).unwrap();
        assert!(p.ends_with("Documents/My Games/Foo/sub/a.sav"));
        assert!(local_path("%WinMyDocuments%My Games/Foo/a.txt", &m, None).is_none(), "pattern must match");
        assert!(local_path("%WinMyDocuments%My Games/Foo/../../x.sav", &m, None).is_none());
        assert!(local_path("%GameInstall%profile/a.dat", &m, None).is_some());
        assert!(local_path("%GameInstall%profile/deep/a.dat", &m, None).is_none(), "not recursive");
        assert!(local_path("%Unknown%a.sav", &m, None).is_none());
        let remote = Path::new("/steam/userdata/1234/5/remote");
        assert_eq!(local_path("slot1.bin", &m, Some(remote)), Some(remote.join("slot1.bin")));
        assert!(local_path("../evil", &m, Some(remote)).is_none());
        assert!(local_path("/abs", &m, Some(remote)).is_none());
        assert!(local_path("slot1.bin", &m, None).is_none());
    }

    #[test]
    fn glob() {
        assert!(glob_match("*.sav", "Slot1.SAV"));
        assert!(glob_match("x?.bin", "xa.bin"));
        assert!(!glob_match("x?.bin", "x.bin"));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("*.sav", "a.save"));
    }

    fn lf(name: &str, sha: &str, time: u64) -> LocalFile {
        LocalFile { name: name.into(), path: PathBuf::from(name), sha: sha.into(), time, size: 1 }
    }

    fn cf(name: &str, sha: &str, time: u64) -> CloudFile {
        CloudFile { name: name.into(), sha: sha.into(), time, size: 1, deleted: false, platforms: PLATFORM_ALL }
    }

    fn base(files: &[(&str, &str)]) -> SyncState {
        let mut s = SyncState::default();
        for (n, sha) in files {
            s.record(n, sha, 1);
        }
        s
    }

    #[test]
    fn change_detection() {
        let b = base(&[("a", "1"), ("b", "1"), ("c", "1"), ("d", "1"), ("e", "1")]);
        let local = [lf("a", "1", 5), lf("b", "2", 5), lf("c", "2", 5), lf("d", "1", 5), lf("new", "9", 5)];
        let cloud =
            [cf("a", "1", 5), cf("b", "1", 5), cf("c", "3", 7), cf("d", "4", 7), cf("e", "1", 7), cf("only", "8", 7)];
        let p = plan(&b, &local, &cloud);
        assert_eq!(p.in_sync, ["a"]);
        assert_eq!(p.uploads, ["b", "new"]);
        assert_eq!(p.conflicts, [Conflict { name: "c".into(), local_time: 5, cloud_time: 7 }]);
        // d: only the cloud changed; e: missing locally (fetched again); only: new in the cloud.
        assert_eq!(p.downloads, ["d", "e", "only"]);
        assert_eq!(p.conflict_times(), (5, 7));
    }

    #[test]
    fn first_sync_without_state() {
        let p = plan(&SyncState::default(), &[lf("a", "1", 1), lf("b", "1", 1)], &[cf("a", "2", 2), cf("b", "1", 2)]);
        assert_eq!(p.conflicts.len(), 1);
        assert_eq!(p.in_sync, ["b"]);
        assert!(p.downloads.is_empty() && p.uploads.is_empty());
    }

    #[test]
    fn cloud_deletions_never_touch_local_files() {
        let b = base(&[("a", "1"), ("b", "1")]);
        let mut gone = cf("a", "1", 2);
        gone.deleted = true;
        let p = plan(&b, &[lf("a", "1", 1), lf("b", "5", 1)], &[gone]);
        assert!(p.downloads.is_empty());
        assert_eq!(p.uploads, ["b"], "a locally changed file is sent again");
        assert_eq!(p.notes.len(), 1);
    }

    #[test]
    fn conflict_decision() {
        let b = base(&[("a", "1"), ("b", "1")]);
        let base_plan = plan(&b, &[lf("a", "2", 5), lf("b", "2", 5)], &[cf("a", "3", 7), cf("b", "3", 8)]);
        let mut keep_local = base_plan.clone();
        keep_local.resolve(Resolution::KeepLocal);
        assert!(keep_local.conflicts.is_empty() && keep_local.downloads.is_empty());
        assert_eq!(keep_local.uploads, ["a", "b"]);
        assert_eq!(keep_local.chosen_uploads, ["a", "b"]);
        let mut keep_cloud = base_plan;
        keep_cloud.resolve(Resolution::KeepCloud);
        assert_eq!(keep_cloud.downloads, ["a", "b"]);
        assert!(keep_cloud.uploads.is_empty() && keep_cloud.chosen_uploads.is_empty());
    }

    #[test]
    fn platform_filter() {
        let mut f = cf("a", "1", 1);
        f.platforms = PLATFORM_WINDOWS;
        assert!(f.wanted_on(Target::Windows) && !f.wanted_on(Target::Linux));
        f.platforms = 0;
        assert!(f.wanted_on(Target::Linux));
    }

    #[test]
    fn state_round_trip() {
        let mut s = SyncState { change: 42, ..Default::default() };
        s.record("%GameInstall%a.sav", "abc", 99);
        let back: SyncState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        let old: SyncState = serde_json::from_str("{}").unwrap();
        assert_eq!(old, SyncState::default());
        // and through a real file
        let dir = std::env::temp_dir().join(format!("sr-cloud-state-{}", std::process::id()));
        let path = dir.join("5.json");
        crate::store::save_json(&path, &s).unwrap();
        assert_eq!(crate::store::load_json::<SyncState>(&path), Some(s));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_finds_matching_files() {
        let dir = std::env::temp_dir().join(format!("sr-cloud-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("saves/sub")).unwrap();
        std::fs::create_dir_all(dir.join("remote/deep")).unwrap();
        std::fs::write(dir.join("saves/a.sav"), b"one").unwrap();
        std::fs::write(dir.join("saves/a.txt"), b"skip").unwrap();
        std::fs::write(dir.join("saves/sub/b.sav"), b"two").unwrap();
        std::fs::write(dir.join("remote/r.bin"), b"r").unwrap();
        std::fs::write(dir.join("remote/deep/s.bin"), b"s").unwrap();
        let m = Mapping {
            token: "%GameInstall%saves/".into(),
            dir: dir.join("saves"),
            patterns: vec!["*.sav".into()],
            recursive: true,
        };
        let (files, notes) = scan(&[m], Some(&dir.join("remote")));
        assert!(notes.is_empty());
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["%GameInstall%saves/a.sav", "%GameInstall%saves/sub/b.sav", "deep/s.bin", "r.bin"]);
        assert_eq!(files[0].sha, sha1_hex(b"one"));
        assert_eq!(files[0].size, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Where `ISteamRemoteStorage` files live and whether we may touch them. Steam's own folders are read-only for
/// us, so with an official Steam root (`official_root` is `Some`) the answer is `None` (skip `remote/`); only a
/// root Kettle made itself (`kettle_dir`, used when there is no official one) is synced.
pub fn remote_dir(official_root: Option<&Path>, kettle_dir: &Path, account_id: u32, appid: u32) -> Option<PathBuf> {
    if official_root.is_some() {
        return None;
    }
    Some(kettle_dir.join("userdata").join(account_id.to_string()).join(appid.to_string()).join("remote"))
}

/// The log line when `remote/` is skipped.
pub const REMOTE_SKIPPED: &str = "Cloud: remote/ belongs to the Steam client, skipped";

#[cfg(test)]
mod remote_tests {
    use super::*;

    #[test]
    fn remote_is_skipped_in_official_roots_only() {
        let kettle = Path::new("/home/u/.local/share/kettle");
        assert_eq!(remote_dir(Some(Path::new("/home/u/.steam/steam")), kettle, 1234, 5), None);
        assert_eq!(
            remote_dir(None, kettle, 1234, 5),
            Some(PathBuf::from("/home/u/.local/share/kettle/userdata/1234/5/remote"))
        );
    }
}
