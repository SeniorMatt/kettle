//! Reader for the official client's binary `appcache/appinfo.vdf` (read-only).
//!
//! The file holds, for every app the client has seen, the same data Steam's PICS service sends: name, type,
//! install dir and the launch entries. Reading it gives us launch configs for all games installed by the
//! official client without any network access.
//!
//! The file can be hundreds of MB on big accounts, so it is never loaded whole: [`AppInfoFile::open`] walks the
//! entry headers once (a few bytes per app) to build an `appid -> (offset, size)` index, and [`AppInfoFile::get`]
//! reads and parses only the requested entry. Formats v27 (0x07564427), v28 and v29 are supported; v29 stores
//! keys as indexes into a string table at the end of the file.

use crate::vdf::Node;
use anyhow::{Context, Result, bail, ensure};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const MAGIC_V27: u32 = 0x0756_4427;
const MAGIC_V28: u32 = 0x0756_4428;
const MAGIC_V29: u32 = 0x0756_4429;

/// Nesting limit so a corrupt file can't overflow the stack.
const MAX_DEPTH: usize = 64;

/// Parsed key/value tree (binary or text). Numbers are widened to `i64`.
#[derive(Debug, Clone, PartialEq)]
pub enum Kv {
    Str(String),
    Int(i64),
    Float(f32),
    Map(Vec<(String, Kv)>),
}

impl Kv {
    /// Case-insensitive child lookup, like [`Node::get`].
    pub fn get(&self, key: &str) -> Option<&Kv> {
        self.children().iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v)
    }

    pub fn path(&self, keys: &[&str]) -> Option<&Kv> {
        keys.iter().try_fold(self, |n, k| n.get(k))
    }

    pub fn children(&self) -> &[(String, Kv)] {
        match self {
            Kv::Map(kids) => kids,
            _ => &[],
        }
    }

    /// String value; numbers are rendered, because PICS text and binary data disagree on types.
    pub fn as_string(&self) -> Option<String> {
        match self {
            Kv::Str(s) => Some(s.clone()),
            Kv::Int(i) => Some(i.to_string()),
            Kv::Float(f) => Some(f.to_string()),
            Kv::Map(_) => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Kv::Int(i) => Some(*i),
            Kv::Str(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    pub fn get_string(&self, key: &str) -> Option<String> {
        self.get(key).and_then(Kv::as_string)
    }

    /// Convert a text-VDF tree (what PICS returns) so both sources share [`AppInfo::from_kv`].
    pub fn from_vdf(node: &Node) -> Kv {
        match node {
            Node::Str(s) => Kv::Str(s.clone()),
            Node::Obj(kids) => Kv::Map(kids.iter().map(|(k, v)| (k.clone(), Kv::from_vdf(v))).collect()),
        }
    }
}

/// One way to start an app (`config/launch/<n>`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaunchEntry {
    /// Key under `launch` ("0", "1", ...); lowest first is what Steam's Play button uses.
    pub id: String,
    pub executable: String,
    pub arguments: String,
    pub workingdir: String,
    /// "default", "none", "vr", "server", "editor", ...; absent on most entries.
    pub kind: Option<String>,
    pub description: Option<String>,
    /// `config/oslist` ("windows", "linux", "macos", comma separated); empty means any.
    pub oslist: Vec<String>,
    /// `config/osarch` ("32" / "64"); `None` means any.
    pub osarch: Option<String>,
    pub betakey: Option<String>,
}

impl LaunchEntry {
    fn from_kv(id: &str, kv: &Kv) -> Option<LaunchEntry> {
        let executable = kv.get_string("executable")?;
        let cfg = kv.get("config");
        let cfg_str = |k: &str| cfg.and_then(|c| c.get_string(k)).filter(|s| !s.is_empty());
        Some(LaunchEntry {
            id: id.to_string(),
            executable,
            arguments: kv.get_string("arguments").unwrap_or_default(),
            workingdir: kv.get_string("workingdir").unwrap_or_default(),
            kind: kv.get_string("type"),
            description: kv.get_string("description").filter(|s| !s.is_empty()),
            oslist: cfg_str("oslist").map(|s| split_list(&s)).unwrap_or_default(),
            osarch: cfg_str("osarch"),
            betakey: cfg_str("betakey"),
        })
    }

    /// Does the entry apply to `os` ("windows" / "linux" / "macos")?
    pub fn runs_on(&self, os: &str) -> bool {
        self.oslist.is_empty() || self.oslist.iter().any(|o| o.eq_ignore_ascii_case(os))
    }

    /// 32-bit-only entries are skipped on 64-bit hosts only when a 64-bit one exists, so this just reports it.
    pub fn is_64bit(&self) -> bool {
        self.osarch.as_deref() != Some("32")
    }

    /// The entry behind the plain "Play" button: no beta branch and a default-ish type.
    pub fn is_default_kind(&self) -> bool {
        self.betakey.is_none() && matches!(self.kind.as_deref(), None | Some("default") | Some("none") | Some(""))
    }
}

fn split_list(s: &str) -> Vec<String> {
    s.split([',', ' ']).map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect()
}

/// What we need from one app's PICS data.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AppInfo {
    pub appid: u32,
    pub name: String,
    /// "game", "tool", "demo", "dlc", ...
    pub app_type: String,
    pub installdir: String,
    /// `common/oslist`, e.g. ["windows", "linux"].
    pub oslist: Vec<String>,
    pub launch: Vec<LaunchEntry>,
    /// Steam Deck verified category: 0 unknown, 1 unsupported, 2 playable, 3 verified.
    pub steam_deck: Option<u8>,
    /// Steam Play tool the developer recommends (`common/steam_play...` is rare; usually `None`).
    pub compat_hint: Option<String>,
}

impl AppInfo {
    /// Build from an `appinfo` root (or its content). Works for both the binary file and PICS text data.
    pub fn from_kv(appid: u32, root: &Kv) -> AppInfo {
        let common = root.get("common");
        let common_str = |k: &str| common.and_then(|c| c.get_string(k)).unwrap_or_default();
        let mut launch: Vec<LaunchEntry> = root
            .path(&["config", "launch"])
            .map(|l| l.children().iter().filter_map(|(id, kv)| LaunchEntry::from_kv(id, kv)).collect())
            .unwrap_or_default();
        launch.sort_by_key(|e| e.id.parse::<u32>().unwrap_or(u32::MAX));
        // An entry without its own `oslist` applies to the OSes the app lists: Windows-only games often leave
        // it out, and without this they would look native.
        let app_oslist = split_list(&common_str("oslist"));
        for e in launch.iter_mut().filter(|e| e.oslist.is_empty()) {
            e.oslist = app_oslist.clone();
        }
        AppInfo {
            appid,
            name: common_str("name"),
            app_type: common_str("type").to_ascii_lowercase(),
            installdir: root.path(&["config", "installdir"]).and_then(Kv::as_string).unwrap_or_default(),
            oslist: app_oslist,
            launch,
            steam_deck: common
                .and_then(|c| c.path(&["steam_deck_compatibility", "category"]))
                .and_then(Kv::as_int)
                .and_then(|n| u8::try_from(n).ok()),
            compat_hint: common.and_then(|c| c.get_string("steam_play_tool")).filter(|s| !s.is_empty()),
        }
    }

    /// Parse PICS text data (`appinfo` section as text VDF).
    pub fn from_vdf(appid: u32, node: &Node) -> AppInfo {
        AppInfo::from_kv(appid, &Kv::from_vdf(node))
    }
}

/// Open `appinfo.vdf`: an index of entries; each [`get`](AppInfoFile::get) parses one app on demand.
pub struct AppInfoFile {
    file: File,
    version: u32,
    strings: Vec<String>,
    index: HashMap<u32, (u64, u32)>,
}

impl AppInfoFile {
    pub fn open(path: &Path) -> Result<AppInfoFile> {
        let mut file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let len = file.metadata()?.len();
        let mut hdr = [0u8; 8];
        file.read_exact(&mut hdr).context("appinfo.vdf is too short")?;
        let magic = u32::from_le_bytes(hdr[..4].try_into().unwrap());
        let (strings, first) = match magic {
            MAGIC_V27 | MAGIC_V28 => (Vec::new(), 8),
            MAGIC_V29 => {
                let mut off = [0u8; 8];
                file.read_exact(&mut off)?;
                let table_at = u64::from_le_bytes(off);
                ensure!(table_at > 16 && table_at < len, "bad string table offset {table_at}");
                (read_string_table(&mut file, table_at)?, 16)
            }
            other => bail!("unsupported appinfo.vdf magic {other:#010x}"),
        };
        let index = build_index(&mut file, first, len)?;
        Ok(AppInfoFile { file, version: magic, strings, index })
    }

    /// Default location inside a Steam root.
    pub fn path_in(steam_root: &Path) -> std::path::PathBuf {
        steam_root.join("appcache/appinfo.vdf")
    }

    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn contains(&self, appid: u32) -> bool {
        self.index.contains_key(&appid)
    }

    pub fn app_ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.index.keys().copied()
    }

    /// Parse one app. `Ok(None)` when the app isn't in the file.
    pub fn get(&mut self, appid: u32) -> Result<Option<AppInfo>> {
        Ok(self.get_kv(appid)?.map(|kv| AppInfo::from_kv(appid, &kv)))
    }

    /// The raw parsed `appinfo` tree of one app (for data [`AppInfo`] does not keep, such as Valve's
    /// compatibility tool tables in app 891390).
    pub fn get_kv(&mut self, appid: u32) -> Result<Option<Kv>> {
        let Some(&(off, size)) = self.index.get(&appid) else { return Ok(None) };
        let mut buf = vec![0u8; size as usize];
        self.file.seek(SeekFrom::Start(off))?;
        self.file.read_exact(&mut buf)?;
        // infostate(4) updated(4) token(8) sha1(20) change(4) [binary sha1(20) from v28]
        let skip = if self.version == MAGIC_V27 { 40 } else { 60 };
        ensure!(buf.len() >= skip, "entry {appid} is truncated");
        Ok(Some(parse_binary_kv(&buf[skip..], &self.strings)?))
    }
}

fn read_string_table(file: &mut File, at: u64) -> Result<Vec<String>> {
    file.seek(SeekFrom::Start(at))?;
    let mut rd = BufReader::new(file);
    let mut n = [0u8; 4];
    rd.read_exact(&mut n)?;
    let count = u32::from_le_bytes(n) as usize;
    // Every string takes at least its terminator, so the count can't exceed the remaining bytes.
    let mut rest = Vec::new();
    rd.read_to_end(&mut rest)?;
    ensure!(count <= rest.len(), "string table claims {count} strings");
    Ok(rest.split(|&b| b == 0).take(count).map(|s| String::from_utf8_lossy(s).into_owned()).collect())
}

/// Walk entry headers only: `appid(4) size(4) data[size]`, ended by appid 0.
fn build_index(file: &mut File, first: u64, len: u64) -> Result<HashMap<u32, (u64, u32)>> {
    file.seek(SeekFrom::Start(first))?;
    let mut rd = BufReader::with_capacity(8 * 1024, file);
    let mut index = HashMap::new();
    let mut pos = first;
    while pos + 8 <= len {
        let mut h = [0u8; 8];
        rd.read_exact(&mut h)?;
        let appid = u32::from_le_bytes(h[..4].try_into().unwrap());
        if appid == 0 {
            break;
        }
        let size = u32::from_le_bytes(h[4..].try_into().unwrap());
        ensure!(pos + 8 + u64::from(size) <= len, "entry {appid} runs past the end of the file");
        index.insert(appid, (pos + 8, size));
        rd.seek_relative(i64::from(size))?;
        pos += 8 + u64::from(size);
    }
    Ok(index)
}

/// Parse binary KeyValues. With a non-empty `strings`, keys are `u32` indexes into it (v29); otherwise
/// they are NUL-terminated. The data is a sequence of pairs, normally one root pair named "appinfo".
pub fn parse_binary_kv(buf: &[u8], strings: &[String]) -> Result<Kv> {
    let mut r = BinReader { buf, pos: 0, strings };
    let mut pairs = r.pairs(0)?;
    if pairs.len() == 1 && pairs[0].0.eq_ignore_ascii_case("appinfo") {
        return Ok(pairs.remove(0).1);
    }
    Ok(Kv::Map(pairs))
}

struct BinReader<'a> {
    buf: &'a [u8],
    pos: usize,
    strings: &'a [String],
}

impl BinReader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.buf.len()).context("unexpected end of data")?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn cstr(&mut self) -> Result<String> {
        let rest = &self.buf[self.pos..];
        let n = rest.iter().position(|&b| b == 0).context("unterminated string")?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.pos += n + 1;
        Ok(s)
    }

    fn key(&mut self) -> Result<String> {
        if self.strings.is_empty() {
            return self.cstr();
        }
        let i = self.u32()? as usize;
        self.strings.get(i).cloned().with_context(|| format!("string index {i} out of range"))
    }

    fn pairs(&mut self, depth: usize) -> Result<Vec<(String, Kv)>> {
        ensure!(depth < MAX_DEPTH, "key/value nesting too deep");
        let mut out = Vec::new();
        while self.pos < self.buf.len() {
            let ty = self.take(1)?[0];
            if ty == 0x08 {
                return Ok(out);
            }
            let key = self.key()?;
            let val = match ty {
                0x00 => Kv::Map(self.pairs(depth + 1)?),
                0x01 => Kv::Str(self.cstr()?),
                0x02 | 0x04 | 0x06 => Kv::Int(i64::from(self.u32()? as i32)),
                0x03 => Kv::Float(f32::from_bits(self.u32()?)),
                0x05 => Kv::Str(self.wstr()?),
                0x07 => Kv::Int(u64::from_le_bytes(self.take(8)?.try_into().unwrap()) as i64),
                0x0A => Kv::Int(i64::from_le_bytes(self.take(8)?.try_into().unwrap())),
                other => bail!("unknown key/value type {other:#04x}"),
            };
            out.push((key, val));
        }
        // Data that ends without the closing 0x08 is tolerated: the last entry of a file may omit it.
        Ok(out)
    }

    fn wstr(&mut self) -> Result<String> {
        let mut units = Vec::new();
        loop {
            let b = self.take(2)?;
            let u = u16::from_le_bytes([b[0], b[1]]);
            if u == 0 {
                return Ok(String::from_utf16_lossy(&units));
            }
            units.push(u);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Binary KV builder for tests (v27/v28 style: inline keys).
    fn s(out: &mut Vec<u8>, key: &str, val: &str) {
        out.push(0x01);
        out.extend(key.bytes().chain([0]));
        out.extend(val.bytes().chain([0]));
    }

    fn map(out: &mut Vec<u8>, key: &str, body: impl FnOnce(&mut Vec<u8>)) {
        out.push(0x00);
        out.extend(key.bytes().chain([0]));
        body(out);
        out.push(0x08);
    }

    fn sample() -> Vec<u8> {
        let mut b = Vec::new();
        map(&mut b, "appinfo", |b| {
            b.push(0x02);
            b.extend(b"appid\0");
            b.extend(730i32.to_le_bytes());
            map(b, "common", |b| {
                s(b, "name", "Test Game");
                s(b, "type", "Game");
                s(b, "oslist", "windows,linux");
                map(b, "steam_deck_compatibility", |b| {
                    b.push(0x02);
                    b.extend(b"category\0");
                    b.extend(3i32.to_le_bytes());
                });
            });
            map(b, "config", |b| {
                s(b, "installdir", "TestGame");
                map(b, "launch", |b| {
                    map(b, "1", |b| {
                        s(b, "executable", "game.exe");
                        s(b, "arguments", "-windowed");
                        map(b, "config", |b| s(b, "oslist", "windows"));
                    });
                    map(b, "0", |b| {
                        s(b, "executable", "game.sh");
                        s(b, "type", "default");
                        map(b, "config", |b| {
                            s(b, "oslist", "linux");
                            s(b, "osarch", "64");
                        });
                    });
                });
            });
        });
        b.push(0x08);
        b
    }

    #[test]
    fn parses_constructed_buffer() {
        let kv = parse_binary_kv(&sample(), &[]).unwrap();
        let info = AppInfo::from_kv(730, &kv);
        assert_eq!(info.name, "Test Game");
        assert_eq!(info.app_type, "game");
        assert_eq!(info.installdir, "TestGame");
        assert_eq!(info.oslist, ["windows", "linux"]);
        assert_eq!(info.steam_deck, Some(3));
        // Sorted by id, whatever the file order.
        assert_eq!(info.launch[0].executable, "game.sh");
        assert!(info.launch[0].runs_on("linux") && !info.launch[0].runs_on("windows"));
        assert!(info.launch[0].is_default_kind());
        assert_eq!(info.launch[1].arguments, "-windowed");
    }

    #[test]
    fn entries_without_oslist_inherit_the_apps() {
        let kv = Kv::Map(vec![
            ("common".into(), Kv::Map(vec![("oslist".into(), Kv::Str("windows".into()))])),
            (
                "config".into(),
                Kv::Map(vec![(
                    "launch".into(),
                    Kv::Map(vec![("0".into(), Kv::Map(vec![("executable".into(), Kv::Str("g.exe".into()))]))]),
                )]),
            ),
        ]);
        let e = &AppInfo::from_kv(1, &kv).launch[0];
        assert!(e.runs_on("windows") && !e.runs_on("linux"));
    }

    #[test]
    fn string_table_keys_and_truncation() {
        // v29: keys are indexes.
        let strings = vec!["appinfo".to_string(), "common".to_string(), "name".to_string()];
        let mut b = vec![0x00];
        b.extend(0u32.to_le_bytes());
        b.push(0x00);
        b.extend(1u32.to_le_bytes());
        b.push(0x01);
        b.extend(2u32.to_le_bytes());
        b.extend(b"Hi\0");
        b.extend([0x08, 0x08, 0x08]);
        let kv = parse_binary_kv(&b, &strings).unwrap();
        assert_eq!(kv.path(&["common", "name"]).and_then(Kv::as_string).as_deref(), Some("Hi"));
        // Bad index and cut-off data are errors, not panics.
        assert!(parse_binary_kv(&[0x01, 9, 0, 0, 0], &strings).is_err());
        assert!(parse_binary_kv(&sample()[..18], &[]).is_err());
    }

    #[test]
    fn reads_file_with_index() {
        let dir = std::env::temp_dir().join(format!("sr-appinfo-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("appinfo.vdf");
        let body = sample();
        let mut entry = vec![0u8; 60]; // header fields (v28)
        entry.extend(&body);
        let mut f = Vec::new();
        f.extend(MAGIC_V28.to_le_bytes());
        f.extend(1u32.to_le_bytes());
        for id in [5u32, 730] {
            f.extend(id.to_le_bytes());
            f.extend((entry.len() as u32).to_le_bytes());
            f.extend(&entry);
        }
        f.extend(0u32.to_le_bytes());
        std::fs::write(&path, f).unwrap();
        let mut file = AppInfoFile::open(&path).unwrap();
        assert_eq!(file.len(), 2);
        assert!(file.get(1).unwrap().is_none());
        assert_eq!(file.get(730).unwrap().unwrap().name, "Test Game");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Timing on a real or synthetic big file: `SR_APPINFO_BENCH=/path cargo test -- --ignored bench --nocapture`.
    #[test]
    #[ignore]
    fn bench_open_and_get() {
        let Some(path) = std::env::var_os("SR_APPINFO_BENCH") else { return };
        let t = std::time::Instant::now();
        let mut f = AppInfoFile::open(Path::new(&path)).unwrap();
        println!("index of {} apps: {:?}", f.len(), t.elapsed());
        let id = f.app_ids().min().unwrap();
        let t = std::time::Instant::now();
        f.get(id).unwrap().unwrap();
        println!("get one: {:?}", t.elapsed());
    }
}
