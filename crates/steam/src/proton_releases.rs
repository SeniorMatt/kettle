//! Catalog of downloadable compatibility tools (GE-Proton, Proton-CachyOS, ...) and install / remove, in the
//! spirit of ProtonUp-Qt.
//!
//! Releases come from the GitHub API (unauthenticated; the JSON is cached for an hour in our cache dir to stay
//! far below the rate limit). Installing downloads the archive into a staging folder inside the target, checks
//! the checksum the release publishes, unpacks it with strict path checks and finally renames the result into
//! place, so a half-installed tool is never visible to Steam or to us.

use crate::{http_dl, store};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::{Duration, SystemTime};

/// How long a cached release list is used before asking GitHub again.
const CACHE_TTL: Duration = Duration::from_secs(3600);
/// Safety limits against decompression bombs: total unpacked bytes and number of entries.
const MAX_UNPACKED: u64 = 8 << 30;
const MAX_ENTRIES: usize = 300_000;

/// A place to get tools from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Source {
    GeProton,
    ProtonCachyOs,
    ProtonEm,
    Luxtorpeda,
    /// Proton-Sarek: Proton builds for old GPUs without Vulkan 1.3 features.
    ProtonSarek,
}

impl Source {
    pub const ALL: [Source; 5] =
        [Source::GeProton, Source::ProtonCachyOs, Source::ProtonEm, Source::Luxtorpeda, Source::ProtonSarek];

    /// Short stable id used in the CLI and in cache file names.
    pub fn id(self) -> &'static str {
        match self {
            Source::GeProton => "ge",
            Source::ProtonCachyOs => "cachyos",
            Source::ProtonEm => "em",
            Source::Luxtorpeda => "luxtorpeda",
            Source::ProtonSarek => "sarek",
        }
    }

    pub fn from_id(id: &str) -> Option<Source> {
        let id = id.to_ascii_lowercase();
        Source::ALL.into_iter().find(|s| s.id() == id || s.label().eq_ignore_ascii_case(&id))
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::GeProton => "GE-Proton",
            Source::ProtonCachyOs => "Proton-CachyOS",
            Source::ProtonEm => "Proton-EM",
            Source::Luxtorpeda => "Luxtorpeda",
            Source::ProtonSarek => "Proton-Sarek",
        }
    }

    /// GitHub `owner/repo`. Proton-EM's old repo name redirects to this one; the API wants the current name
    /// because we don't follow redirects for API calls.
    pub fn repo(self) -> &'static str {
        match self {
            Source::GeProton => "GloriousEggroll/proton-ge-custom",
            Source::ProtonCachyOs => "CachyOS/proton-cachyos",
            Source::ProtonEm => "BananaWorks07/Proton",
            Source::Luxtorpeda => "luxtorpeda-dev/luxtorpeda",
            Source::ProtonSarek => "pythonlover02/Proton-Sarek",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Source::GeProton => "Valve's Proton plus extra patches and codecs; the most popular custom build",
            Source::ProtonCachyOs => "Proton tuned by CachyOS (fast builds for modern CPUs)",
            Source::ProtonEm => "Proton with fixes for new Windows games (HDR, Wayland)",
            Source::Luxtorpeda => "Runs some games with their native open-source engines",
            Source::ProtonSarek => "Proton for older GPUs without full Vulkan 1.3",
        }
    }
}

/// One downloadable file of a release.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// `sha256` published by GitHub for the asset (hex, no prefix), when it has one.
    pub sha256: Option<String>,
}

/// A release we can install: the archive for this machine and its checksum file, if any.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Release {
    pub source_id: String,
    pub tag: String,
    pub published: String,
    pub prerelease: bool,
    pub archive: Asset,
    pub checksum: Option<Asset>,
}

impl Release {
    pub fn source(&self) -> Option<Source> {
        Source::from_id(&self.source_id)
    }

    /// Folder name the archive normally unpacks to (the archive name without extension).
    pub fn archive_stem(&self) -> &str {
        strip_archive_ext(&self.archive.name)
    }
}

fn strip_archive_ext(name: &str) -> &str {
    [".tar.gz", ".tar.xz", ".tar.zst", ".tgz"].iter().find_map(|e| name.strip_suffix(e)).unwrap_or(name)
}

// ---- CPU level -------------------------------------------------------------------------------------------

/// x86-64 microarchitecture level, as far as asset names distinguish them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CpuLevel {
    V1,
    V2,
    V3,
    V4,
}

impl CpuLevel {
    fn tag(self) -> Option<&'static str> {
        match self {
            CpuLevel::V1 => None,
            CpuLevel::V2 => Some("x86_64_v2"),
            CpuLevel::V3 => Some("x86_64_v3"),
            CpuLevel::V4 => Some("x86_64_v4"),
        }
    }
}

/// Level from the `flags` line of `/proc/cpuinfo` (flag names as the kernel prints them).
pub fn cpu_level_from_flags(flags: &str) -> CpuLevel {
    let has = |wanted: &[&str]| wanted.iter().all(|w| flags.split_whitespace().any(|f| f == *w));
    let v2 = has(&["cx16", "popcnt", "sse4_1", "sse4_2", "ssse3"]);
    // v3 adds AVX2, BMI1/2, FMA, F16C, LZCNT (kernel name `abm`) and MOVBE.
    let v3 = v2 && has(&["avx", "avx2", "bmi1", "bmi2", "fma", "f16c", "abm", "movbe", "xsave"]);
    let v4 = v3 && has(&["avx512f", "avx512bw", "avx512cd", "avx512dq", "avx512vl"]);
    match (v2, v3, v4) {
        (_, _, true) => CpuLevel::V4,
        (_, true, _) => CpuLevel::V3,
        (true, _, _) => CpuLevel::V2,
        _ => CpuLevel::V1,
    }
}

/// The level of this machine (read once). Non-Linux and non-x86 hosts count as V1.
pub fn cpu_level() -> CpuLevel {
    static LEVEL: std::sync::OnceLock<CpuLevel> = std::sync::OnceLock::new();
    *LEVEL.get_or_init(|| {
        std::fs::read_to_string("/proc/cpuinfo")
            .ok()
            .and_then(|s| s.lines().find(|l| l.starts_with("flags")).map(cpu_level_from_flags))
            .unwrap_or(CpuLevel::V1)
    })
}

// ---- Release listing ---------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    published_at: Option<String>,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    digest: Option<String>,
}

impl From<&GhAsset> for Asset {
    fn from(a: &GhAsset) -> Asset {
        Asset {
            name: a.name.clone(),
            url: a.browser_download_url.clone(),
            size: a.size,
            sha256: a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")).map(str::to_string),
        }
    }
}

fn is_archive(name: &str) -> bool {
    [".tar.gz", ".tar.xz", ".tar.zst", ".tgz"].iter().any(|e| name.ends_with(e))
}

/// The archive to install from a release's assets, plus its checksum file.
///
/// Archives for other CPU architectures (`aarch64`, `arm64`) are skipped, and so are `async` variants when a
/// plain one exists. Where builds exist per x86-64 level (CachyOS), the highest level this CPU supports wins and
/// plain `x86_64` is the fallback.
fn pick_assets(assets: &[GhAsset], level: CpuLevel) -> Option<(&GhAsset, Option<&GhAsset>)> {
    let rank = |a: &GhAsset| -> Option<(u8, u8)> {
        let n = a.name.to_ascii_lowercase();
        if !is_archive(&n) || n.contains("aarch64") || n.contains("arm64") {
            return None;
        }
        let lvl = [CpuLevel::V4, CpuLevel::V3, CpuLevel::V2]
            .into_iter()
            .find(|l| l.tag().is_some_and(|t| n.contains(t)))
            .unwrap_or(CpuLevel::V1);
        (lvl <= level).then_some((lvl as u8, u8::from(!n.contains("async"))))
    };
    let archive = assets.iter().filter_map(|a| rank(a).map(|r| (r, a))).max_by_key(|(r, _)| *r)?.1;
    let checksum = ["sha512sum", "sha512", "sha256sum", "sha256"]
        .iter()
        .find_map(|ext| assets.iter().find(|a| a.name == format!("{}.{ext}", archive.name)))
        // GE and CachyOS name it `<archive minus .tar.gz>.sha512sum`.
        .or_else(|| {
            let stem = strip_archive_ext(&archive.name);
            ["sha512sum", "sha512", "sha256sum", "sha256"]
                .iter()
                .find_map(|ext| assets.iter().find(|a| a.name == format!("{stem}.{ext}")))
        });
    Some((archive, checksum))
}

fn to_release(source: Source, r: &GhRelease, level: CpuLevel) -> Option<Release> {
    if r.draft {
        return None;
    }
    let (archive, checksum) = pick_assets(&r.assets, level)?;
    Some(Release {
        source_id: source.id().to_string(),
        tag: r.tag_name.clone(),
        published: r.published_at.clone().unwrap_or_default(),
        prerelease: r.prerelease,
        archive: archive.into(),
        checksum: checksum.map(Asset::from),
    })
}

fn cache_path(source: Source) -> PathBuf {
    store::cache_dir().join("proton-releases").join(format!("{}.json", source.id()))
}

fn cache_fresh(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_some_and(|age| age < CACHE_TTL)
}

/// Releases of a source, newest first, as installable for this machine. Uses the hourly cache unless
/// `refresh`; when GitHub can't be reached an older cache is used rather than failing.
pub fn releases(source: Source, refresh: bool) -> Result<Vec<Release>> {
    let path = cache_path(source);
    let cached: Option<Vec<GhRelease>> = store::load_json(&path);
    let list = match cached {
        Some(c) if !refresh && cache_fresh(&path) => c,
        stale => match fetch_github(source) {
            Ok(fresh) => {
                // A cache write failure only costs a refetch later.
                let _ = store::save_json(&path, &fresh);
                fresh
            }
            Err(e) => stale.ok_or(e)?,
        },
    };
    let level = cpu_level();
    Ok(list.iter().filter_map(|r| to_release(source, r, level)).collect())
}

fn fetch_github(source: Source) -> Result<Vec<GhRelease>> {
    let url = format!("https://api.github.com/repos/{}/releases?per_page=20", source.repo());
    let mut resp =
        http_dl::agent().get(&url).header("Accept", "application/vnd.github+json").call().with_context(|| {
            format!("couldn't list {} releases (GitHub may be rate limiting; try later)", source.label())
        })?;
    resp.body_mut().read_json().context("unexpected GitHub response")
}

/// `tag` of a source, or the newest stable release for "latest".
pub fn find_release(source: Source, tag: &str, refresh: bool) -> Result<Release> {
    let all = releases(source, refresh)?;
    let found = if tag == "latest" {
        all.iter().find(|r| !r.prerelease).or(all.first())
    } else {
        all.iter().find(|r| r.tag == tag)
    };
    found.cloned().with_context(|| format!("no release {tag:?} for {} among the latest 20", source.label()))
}

// ---- Install -----------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Download,
    Verify,
    Extract,
}

/// Progress report: `done` of `total` bytes of the current phase (extract counts compressed bytes read).
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

#[derive(Debug, Clone)]
pub struct Installed {
    /// Folder name inside the target (the tool's name for Steam).
    pub name: String,
    pub dir: PathBuf,
    /// False when the release published no checksum at all.
    pub verified: bool,
}

/// Typed so the UI can offer "remove first" or just say "cancelled".
#[derive(Debug)]
pub enum InstallError {
    Cancelled,
    AlreadyInstalled(PathBuf),
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Cancelled => write!(f, "installation cancelled"),
            InstallError::AlreadyInstalled(p) => write!(f, "{} is already installed", p.display()),
        }
    }
}

impl std::error::Error for InstallError {}

/// Removes the staging folder however `install` ends.
struct Staging(PathBuf);

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Download, verify, unpack and move `release` into `target` (a `compatibilitytools.d` folder, created if
/// missing). Blocking; run it on a worker thread. `cancel` is checked about every 64 KiB.
pub fn install(
    release: &Release,
    target: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Installed> {
    std::fs::create_dir_all(target).with_context(|| format!("creating {}", target.display()))?;
    let staging = target.join(format!(".sr-staging-{}-{}", std::process::id(), release.tag.replace(['/', '\\'], "_")));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)?;
    let _guard = Staging(staging.clone());

    let expected = expected_hash(release)?;
    let archive = staging.join(&release.archive.name);
    ensure!(release.archive.name == file_name_only(&release.archive.name), "unsafe archive name");
    let actual = download(&release.archive, &archive, expected.as_ref().map(|e| e.algo), cancel, progress)?;
    if let Some(exp) = &expected {
        progress(Progress { phase: Phase::Verify, done: 1, total: 1 });
        ensure!(
            actual.as_deref() == Some(exp.hex.as_str()),
            "checksum mismatch for {} (expected {}, got {})",
            release.archive.name,
            exp.hex,
            actual.unwrap_or_default()
        );
    }

    let out = staging.join("out");
    std::fs::create_dir(&out)?;
    extract_archive(&archive, &out, cancel, progress)?;
    std::fs::remove_file(&archive)?;

    let name = unpacked_name(&out, release.archive_stem())?;
    let dest = target.join(&name);
    if dest.exists() {
        return Err(InstallError::AlreadyInstalled(dest).into());
    }
    std::fs::rename(out.join(&name), &dest).with_context(|| format!("moving into {}", dest.display()))?;
    Ok(Installed { name, dir: dest, verified: expected.is_some() })
}

fn file_name_only(name: &str) -> &str {
    Path::new(name).file_name().and_then(|n| n.to_str()).unwrap_or("")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Algo {
    Sha256,
    Sha512,
}

struct Expected {
    algo: Algo,
    hex: String,
}

/// The checksum to verify against: the release's checksum file when it has one, else GitHub's asset digest.
fn expected_hash(release: &Release) -> Result<Option<Expected>> {
    if let Some(cs) = &release.checksum {
        let text = http_dl::get_text(&cs.url, 64 * 1024)?;
        return Ok(Some(parse_checksum_file(&text, &release.archive.name).context("unreadable checksum file")?));
    }
    Ok(release.archive.sha256.as_ref().map(|h| Expected { algo: Algo::Sha256, hex: h.to_ascii_lowercase() }))
}

/// `<hex>  <file>` lines (coreutils format) or a bare hash. The line naming our archive wins, else the first.
fn parse_checksum_file(text: &str, archive: &str) -> Option<Expected> {
    let parse = |line: &str| -> Option<(Expected, Option<String>)> {
        let mut it = line.split_whitespace();
        let hex = it.next()?.to_ascii_lowercase();
        let algo = match hex.len() {
            128 => Algo::Sha512,
            64 => Algo::Sha256,
            _ => return None,
        };
        hex.bytes()
            .all(|b| b.is_ascii_hexdigit())
            .then(|| (Expected { algo, hex }, it.next().map(|n| n.trim_start_matches('*').to_string())))
    };
    let all: Vec<_> = text.lines().filter_map(parse).collect();
    let pos = all.iter().position(|(_, n)| n.as_deref().is_some_and(|n| file_name_only(n) == archive)).unwrap_or(0);
    all.into_iter().nth(pos).map(|(e, _)| e)
}

/// Stream `asset` to `dest`, hashing on the way. Returns the hex digest when `algo` is given.
fn download(
    asset: &Asset,
    dest: &Path,
    algo: Option<Algo>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<Option<String>> {
    let mut resp = http_dl::agent().get(&asset.url).call().with_context(|| format!("downloading {}", asset.name))?;
    let total = resp.body().content_length().unwrap_or(asset.size).max(1);
    let mut body = resp.body_mut().as_reader();
    let mut out = BufWriter::new(std::fs::File::create(dest)?);
    let mut h256 = Sha256::new();
    let mut h512 = Sha512::new();
    let mut buf = vec![0u8; 64 * 1024];
    let (mut done, mut last_report) = (0u64, 0u64);
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled.into());
        }
        let n = body.read(&mut buf).with_context(|| format!("downloading {}", asset.name))?;
        if n == 0 {
            break;
        }
        match algo {
            Some(Algo::Sha256) => h256.update(&buf[..n]),
            Some(Algo::Sha512) => h512.update(&buf[..n]),
            None => {}
        }
        out.write_all(&buf[..n])?;
        done += n as u64;
        if done - last_report >= 512 * 1024 {
            last_report = done;
            progress(Progress { phase: Phase::Download, done, total });
        }
    }
    out.flush()?;
    progress(Progress { phase: Phase::Download, done, total: done.max(1) });
    Ok(algo.map(|a| match a {
        Algo::Sha256 => hex(&h256.finalize()),
        Algo::Sha512 => hex(&h512.finalize()),
    }))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The single top-level folder of the unpacked archive; archives without one are wrapped in `fallback`.
fn unpacked_name(out: &Path, fallback: &str) -> Result<String> {
    let entries: Vec<_> = std::fs::read_dir(out)?.flatten().collect();
    if let [only] = entries.as_slice()
        && only.path().is_dir()
    {
        return Ok(only.file_name().to_string_lossy().into_owned());
    }
    ensure!(!entries.is_empty(), "the archive was empty");
    let name = file_name_only(fallback).to_string();
    ensure!(!name.is_empty(), "unsafe tool name");
    let wrapper = out.join(&name);
    let tmp = out.join(".wrap");
    std::fs::create_dir(&tmp)?;
    for e in entries {
        std::fs::rename(e.path(), tmp.join(e.file_name()))?;
    }
    std::fs::rename(&tmp, &wrapper)?;
    Ok(name)
}

// ---- Extraction --------------------------------------------------------------------------------------------

/// Counts bytes read so progress can be reported while a decoder thread consumes the file.
struct Counting<R> {
    inner: R,
    count: std::sync::Arc<AtomicU64>,
}

impl<R: Read> Read for Counting<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Feeds decoded bytes from a decoder thread into the tar reader.
struct PipeReader {
    rx: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    cur: Vec<u8>,
    pos: usize,
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.cur.len() {
            match self.rx.recv() {
                Ok(Ok(chunk)) => (self.cur, self.pos) = (chunk, 0),
                Ok(Err(e)) => return Err(e),
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.cur.len() - self.pos);
        buf[..n].copy_from_slice(&self.cur[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct PipeWriter(mpsc::SyncSender<std::io::Result<Vec<u8>>>);

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // The reader hanging up (extraction failed or was cancelled) stops the decoder thread too.
        self.0.send(Ok(buf.to_vec())).map_err(|_| std::io::Error::other("extraction stopped"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Unpack `archive` (.tar.gz / .tar.xz / .tar.zst) into `dest` with strict checks, see [`unpack_tar`].
pub fn extract_archive(
    archive: &Path,
    dest: &Path,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let total = std::fs::metadata(archive)?.len().max(1);
    let read = std::sync::Arc::new(AtomicU64::new(0));
    let file = Counting { inner: std::fs::File::open(archive)?, count: read.clone() };
    let name = archive.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let mut report =
        |_: &str| progress(Progress { phase: Phase::Extract, done: read.load(Ordering::Relaxed).min(total), total });

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        unpack_tar(flate2::read::GzDecoder::new(BufReader::new(file)), dest, cancel, &mut report)
    } else if name.ends_with(".tar.zst") {
        let dec = ruzstd::decoding::StreamingDecoder::new(BufReader::new(file)).context("not a zstd archive")?;
        unpack_tar(dec, dest, cancel, &mut report)
    } else if name.ends_with(".tar.xz") {
        // The pure-Rust xz decoder writes into a `Write`, so it runs in a thread feeding a pipe.
        let (tx, rx) = mpsc::sync_channel(16);
        let worker = std::thread::spawn(move || {
            let mut input = BufReader::new(file);
            let mut output = BufWriter::with_capacity(256 * 1024, PipeWriter(tx.clone()));
            let res = lzma_rs::xz_decompress(&mut input, &mut output).map_err(|e| std::io::Error::other(e.to_string()));
            let res = res.and_then(|()| output.flush());
            if let Err(e) = res {
                let _ = tx.send(Err(e));
            }
        });
        let res = unpack_tar(PipeReader { rx, cur: Vec::new(), pos: 0 }, dest, cancel, &mut report);
        // The reader is gone by now (moved into unpack_tar), so the worker stops on its next write.
        let _ = worker.join();
        res
    } else {
        bail!("unsupported archive type: {name}")
    }
}

/// Is `p` a plain relative path (no root, drive, or `..`)?
fn is_plain_relative(p: &Path) -> bool {
    p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Would a symlink at `entry` pointing to `target` stay inside the archive root? Checked lexically.
fn symlink_stays_inside(entry: &Path, target: &Path) -> bool {
    if target.is_absolute() {
        return false;
    }
    let mut depth = entry.components().filter(|c| matches!(c, Component::Normal(_))).count() as i64 - 1;
    for c in target.components() {
        match c {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => depth -= 1,
            Component::CurDir => {}
            _ => return false,
        }
        if depth < 0 {
            return false;
        }
    }
    true
}

/// Unpack a tar stream into `dest`. Rejects (as errors, nothing is skipped silently):
/// absolute paths and `..`; symlinks or hard links leading outside the tree; device nodes and FIFOs;
/// archives claiming more than 8 GiB or 300k entries. `tar`'s own `unpack_in` additionally refuses to write
/// through a symlink that points out of `dest`.
pub fn unpack_tar<R: Read>(reader: R, dest: &Path, cancel: &AtomicBool, report: &mut dyn FnMut(&str)) -> Result<()> {
    let mut ar = tar::Archive::new(reader);
    ar.set_preserve_permissions(true);
    ar.set_unpack_xattrs(false);
    ar.set_overwrite(false);
    let (mut bytes, mut count) = (0u64, 0usize);
    for entry in ar.entries()? {
        if cancel.load(Ordering::Relaxed) {
            return Err(InstallError::Cancelled.into());
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        ensure!(is_plain_relative(&path), "archive entry has an unsafe path: {}", path.display());
        let ty = entry.header().entry_type();
        match ty {
            t if t.is_file() || t.is_dir() => {}
            t if t.is_symlink() || t.is_hard_link() => {
                let link = entry.link_name()?.context("link without a target")?.into_owned();
                let ok = if t.is_hard_link() { is_plain_relative(&link) } else { symlink_stays_inside(&path, &link) };
                ensure!(ok, "archive link {} points outside the tool folder: {}", path.display(), link.display());
            }
            t if t.is_pax_global_extensions()
                || t.is_pax_local_extensions()
                || t.is_gnu_longname()
                || t.is_gnu_longlink() =>
            {
                continue;
            }
            _ => bail!("archive entry {} is a special file", path.display()),
        }
        bytes += entry.header().size().unwrap_or(0);
        count += 1;
        ensure!(bytes <= MAX_UNPACKED && count <= MAX_ENTRIES, "archive is unreasonably large");
        entry.unpack_in(dest)?;
        if count % 256 == 0 {
            report("");
        }
    }
    report("");
    Ok(())
}

// ---- Remove ------------------------------------------------------------------------------------------------

/// Is `dir` a tool folder directly inside a `compatibilitytools.d`?
pub fn is_removable_tool(dir: &Path) -> bool {
    let in_tools_d = dir.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "compatibilitytools.d");
    let is_real_dir = std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir());
    let looks_like_tool = ["compatibilitytool.vdf", "toolmanifest.vdf", "proton"].iter().any(|f| dir.join(f).is_file());
    in_tools_d && is_real_dir && looks_like_tool
}

/// Delete an installed tool. Refuses anything that isn't a tool folder inside a `compatibilitytools.d`.
pub fn remove(dir: &Path) -> Result<()> {
    ensure!(
        is_removable_tool(dir),
        "{} is not a compatibility tool inside a compatibilitytools.d folder",
        dir.display()
    );
    std::fs::remove_dir_all(dir).with_context(|| format!("removing {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> GhAsset {
        GhAsset { name: name.into(), size: 1, browser_download_url: format!("https://x/{name}"), digest: None }
    }

    #[test]
    fn cpu_levels() {
        let v1 = "fpu sse2 lm";
        let v2 = "fpu sse2 cx16 popcnt sse4_1 sse4_2 ssse3";
        let v3 = format!("{v2} avx avx2 bmi1 bmi2 fma f16c abm movbe xsave");
        assert_eq!(cpu_level_from_flags(v1), CpuLevel::V1);
        assert_eq!(cpu_level_from_flags(v2), CpuLevel::V2);
        assert_eq!(cpu_level_from_flags(&v3), CpuLevel::V3);
        // AVX2 without FMA is not v3.
        assert_eq!(cpu_level_from_flags(&format!("{v2} avx avx2 bmi1 bmi2")), CpuLevel::V2);
        let v4 = format!("{v3} avx512f avx512bw avx512cd avx512dq avx512vl");
        assert_eq!(cpu_level_from_flags(&v4), CpuLevel::V4);
    }

    #[test]
    fn cachyos_asset_choice() {
        let n = "proton-cachyos-11.0-20261015-slr";
        let assets: Vec<GhAsset> = ["arm64", "x86_64", "x86_64_v3"]
            .iter()
            .flat_map(|l| [format!("{n}-{l}.sha512sum"), format!("{n}-{l}.tar.xz")])
            .map(|s| asset(&s))
            .collect();
        let (a, c) = pick_assets(&assets, CpuLevel::V3).unwrap();
        assert_eq!(
            (a.name.as_str(), c.map(|c| c.name.as_str())),
            (&*format!("{n}-x86_64_v3.tar.xz"), Some(&*format!("{n}-x86_64_v3.sha512sum")))
        );
        assert_eq!(pick_assets(&assets, CpuLevel::V2).unwrap().0.name, format!("{n}-x86_64.tar.xz"));
        assert_eq!(pick_assets(&assets, CpuLevel::V4).unwrap().0.name, format!("{n}-x86_64_v3.tar.xz"));
    }

    #[test]
    fn other_sources_asset_choice() {
        let ge: Vec<_> =
            ["GE-Proton11-12-aarch64.tar.gz", "GE-Proton11-12-x86_64.tar.gz", "GE-Proton11-12-x86_64.sha512sum"]
                .iter()
                .map(|n| asset(n))
                .collect();
        let (a, c) = pick_assets(&ge, CpuLevel::V3).unwrap();
        assert_eq!(a.name, "GE-Proton11-12-x86_64.tar.gz");
        assert_eq!(c.unwrap().name, "GE-Proton11-12-x86_64.sha512sum");
        let lux: Vec<_> =
            ["luxtorpeda-v76.3.0.tar.xz", "luxtorpeda-v76.3.0.tar.xz.sha512"].iter().map(|n| asset(n)).collect();
        assert_eq!(pick_assets(&lux, CpuLevel::V1).unwrap().1.unwrap().name, "luxtorpeda-v76.3.0.tar.xz.sha512");
        let sarek: Vec<_> =
            ["Proton-Sarek10-29-async.tar.gz", "Proton-Sarek10-29.tar.gz"].iter().map(|n| asset(n)).collect();
        assert_eq!(pick_assets(&sarek, CpuLevel::V1).unwrap().0.name, "Proton-Sarek10-29.tar.gz");
        assert!(pick_assets(&[asset("notes.txt")], CpuLevel::V1).is_none());
    }

    #[test]
    fn checksum_files() {
        let h = "a".repeat(128);
        let e = parse_checksum_file(&format!("{h}  GE-Proton11-12-x86_64.tar.gz\n"), "GE-Proton11-12-x86_64.tar.gz")
            .unwrap();
        assert!(e.algo == Algo::Sha512 && e.hex == h);
        let h2 = "b".repeat(64);
        let e = parse_checksum_file(&format!("{h2} *other\n{h2} mine.tar.xz"), "mine.tar.xz").unwrap();
        assert!(e.algo == Algo::Sha256);
        assert!(parse_checksum_file("not a hash", "x").is_none());
    }

    // ---- extraction ----

    fn tar_bytes(build: impl FnOnce(&mut tar::Builder<Vec<u8>>)) -> Vec<u8> {
        let mut b = tar::Builder::new(Vec::new());
        build(&mut b);
        b.into_inner().unwrap()
    }

    /// Write a header with an arbitrary path (the safe `append_data` API refuses `..`).
    fn raw_entry(b: &mut tar::Builder<Vec<u8>>, path: &str, ty: tar::EntryType, link: Option<&str>, data: &[u8]) {
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(ty);
        h.set_size(data.len() as u64);
        h.set_mode(if ty == tar::EntryType::Directory { 0o755 } else { 0o644 });
        {
            let name = &mut h.as_old_mut().name;
            name.fill(0);
            name[..path.len()].copy_from_slice(path.as_bytes());
        }
        if let Some(l) = link {
            h.set_link_name(l).unwrap();
        }
        h.set_cksum();
        b.append(&h, data).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sr-unpack-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn unpack(bytes: Vec<u8>, dest: &Path) -> Result<()> {
        unpack_tar(&bytes[..], dest, &AtomicBool::new(false), &mut |_| {})
    }

    #[test]
    fn unpacks_normal_archive() {
        let d = scratch("ok");
        let t = tar_bytes(|b| {
            raw_entry(b, "tool/", tar::EntryType::Directory, None, b"");
            raw_entry(b, "tool/proton", tar::EntryType::Regular, None, b"#!/bin/sh");
            raw_entry(b, "tool/link", tar::EntryType::Symlink, Some("proton"), b"");
            raw_entry(b, "tool/files/../ok", tar::EntryType::Regular, None, b"x");
        });
        // The last entry contains `..` and must be refused even though it resolves inside.
        assert!(unpack(t, &d).is_err());
        let t = tar_bytes(|b| {
            raw_entry(b, "tool/", tar::EntryType::Directory, None, b"");
            raw_entry(b, "tool/proton", tar::EntryType::Regular, None, b"#!/bin/sh");
            raw_entry(b, "tool/link", tar::EntryType::Symlink, Some("proton"), b"");
        });
        // The refused archive may have left files behind, so unpack the good one into a fresh folder.
        let d2 = scratch("ok2");
        unpack(t, &d2).unwrap();
        assert!(d2.join("tool/proton").is_file() && d2.join("tool/link").exists());
        std::fs::remove_dir_all(&d).unwrap();
        std::fs::remove_dir_all(&d2).unwrap();
    }

    #[test]
    fn rejects_traversal_and_bad_links() {
        let d = scratch("bad");
        let cases: Vec<(&str, tar::EntryType, Option<&str>)> = vec![
            ("../evil", tar::EntryType::Regular, None),
            ("/etc/evil", tar::EntryType::Regular, None),
            ("a/../../evil", tar::EntryType::Regular, None),
            ("tool/l", tar::EntryType::Symlink, Some("/etc")),
            ("tool/l", tar::EntryType::Symlink, Some("../../x")),
            ("tool/l", tar::EntryType::Link, Some("../x")),
            ("dev", tar::EntryType::Char, None),
        ];
        for (path, ty, link) in cases {
            let t = tar_bytes(|b| raw_entry(b, path, ty, link, b""));
            assert!(unpack(t, &d).is_err(), "should reject {path} {ty:?} {link:?}");
        }
        // Nothing escaped or was created.
        assert!(!d.join("evil").exists() && !d.parent().unwrap().join("evil").exists());
        // A symlink inside the tree is fine.
        let t = tar_bytes(|b| raw_entry(b, "tool/l", tar::EntryType::Symlink, Some("../other/x"), b""));
        unpack(t, &d).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn symlink_depth() {
        assert!(symlink_stays_inside(Path::new("a/b/c"), Path::new("../x")));
        assert!(!symlink_stays_inside(Path::new("a/c"), Path::new("../../x")));
        assert!(symlink_stays_inside(Path::new("c"), Path::new("a/../b")));
    }

    #[test]
    fn gz_roundtrip_and_remove_guard() {
        let d = scratch("gz");
        let t = tar_bytes(|b| {
            raw_entry(b, "GE-Proton1/proton", tar::EntryType::Regular, None, b"x");
            raw_entry(b, "GE-Proton1/compatibilitytool.vdf", tar::EntryType::Regular, None, b"");
        });
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        enc.write_all(&t).unwrap();
        let archive = d.join("GE-Proton1.tar.gz");
        std::fs::write(&archive, enc.finish().unwrap()).unwrap();
        let out = d.join("compatibilitytools.d");
        std::fs::create_dir(&out).unwrap();
        extract_archive(&archive, &out, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(unpacked_name(&out, "x").unwrap(), "GE-Proton1");
        // Remove only works on tool folders directly inside compatibilitytools.d.
        assert!(remove(&d).is_err());
        assert!(remove(&out).is_err());
        remove(&out.join("GE-Proton1")).unwrap();
        assert!(!out.join("GE-Proton1").exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn cancel_stops_extraction() {
        let d = scratch("cancel");
        let t = tar_bytes(|b| raw_entry(b, "t/f", tar::EntryType::Regular, None, b"x"));
        let r = unpack_tar(&t[..], &d, &AtomicBool::new(true), &mut |_| {});
        assert!(matches!(r.unwrap_err().downcast_ref::<InstallError>(), Some(InstallError::Cancelled)));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
