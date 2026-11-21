//! Writing depots to disk: preallocate files, verify what is already there,
//! download only missing chunks in parallel, then fix up modes, symlinks and
//! files that disappeared upstream. Blocking code, meant for a worker thread.

use super::cdn::Cdn;
use super::crypto::DepotKey;
use super::manifest::{Chunk, FileEntry, Manifest, checksum};
use super::{CancelToken, Phase, bandwidth, compress, crypto};
use anyhow::{Context, Result, ensure};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Everything needed to bring one depot to its new manifest.
pub struct DepotPlan {
    pub key: DepotKey,
    pub new: Manifest,
    /// What is installed now, when known (from the depot cache).
    pub old: Option<Manifest>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub files: u64,
    pub bytes: u64,
    pub chunks_downloaded: u64,
    pub bytes_downloaded: u64,
    pub chunks_reused: u64,
    pub files_removed: u64,
}

pub struct Settings<'a> {
    pub dir: &'a Path,
    pub cdn: &'a Cdn,
    pub cancel: &'a CancelToken,
    pub workers: usize,
    /// Hash every file, even ones the old manifest says are unchanged.
    pub verify: bool,
}

/// `(plan, file)` indexes of one file.
type FileRef = (usize, usize);

/// `(plan, file, chunk)` indexes of one chunk to fetch.
type ChunkRef = (usize, usize, usize);

/// Run the whole install. `report(phase, done, total)` is called about ten times a second.
pub fn apply(s: &Settings, plans: &[DepotPlan], report: &mut (dyn FnMut(Phase, u64, u64) + Send)) -> Result<Stats> {
    let mut stats = Stats::default();
    let (to_check, mut wanted) = prepare(s, plans, &mut stats)?;

    if !to_check.is_empty() {
        let total: u64 = to_check.iter().map(|&(p, f)| plans[p].new.files[f].size).sum();
        let bad = Mutex::new(Vec::new());
        let done = AtomicU64::new(0);
        parallel(
            &to_check,
            s,
            || report(Phase::Verifying, done.load(Ordering::Relaxed), total),
            |&(p, f)| {
                let file = &plans[p].new.files[f];
                let missing = check_file(&s.dir.join(&file.path), file, &done)?;
                bad.lock().unwrap().extend(missing.into_iter().map(|c| (p, f, c)));
                Ok(())
            },
        )?;
        wanted.extend(bad.into_inner().unwrap());
    }
    wanted.sort_unstable();
    let total_chunks: usize = plans.iter().flat_map(|p| &p.new.files).map(|f| f.chunks.len()).sum();
    stats.chunks_reused = (total_chunks - wanted.len()) as u64;

    let total: u64 = wanted.iter().map(|&(p, f, c)| u64::from(plans[p].new.files[f].chunks[c].size)).sum();
    let done = AtomicU64::new(0);
    report(Phase::Downloading, 0, total);
    parallel(
        &wanted,
        s,
        || report(Phase::Downloading, done.load(Ordering::Relaxed), total),
        |&(p, f, c)| {
            let plan = &plans[p];
            let (file, chunk) = (&plan.new.files[f], &plan.new.files[f].chunks[c]);
            fetch_chunk_to(s, plan, file, chunk)?;
            done.fetch_add(u64::from(chunk.size), Ordering::Relaxed);
            Ok(())
        },
    )?;
    report(Phase::Downloading, total, total);
    stats.chunks_downloaded = wanted.len() as u64;
    stats.bytes_downloaded = total;

    finish_files(s.dir, plans, &mut stats)?;
    Ok(stats)
}

/// Create directories and files, work out which files need hashing and which
/// chunks are certainly missing (brand-new files need no hashing at all).
fn prepare(s: &Settings, plans: &[DepotPlan], stats: &mut Stats) -> Result<(Vec<FileRef>, Vec<ChunkRef>)> {
    let (mut to_check, mut wanted) = (Vec::new(), Vec::new());
    for (p, plan) in plans.iter().enumerate() {
        let old: HashMap<&str, &FileEntry> =
            plan.old.iter().flat_map(|m| &m.files).map(|f| (f.path.as_str(), f)).collect();
        for (f, file) in plan.new.files.iter().enumerate() {
            let path = s.dir.join(&file.path);
            if file.is_dir() {
                std::fs::create_dir_all(&path)?;
                continue;
            }
            if file.link.is_some() {
                continue; // made in finish_files
            }
            stats.files += 1;
            stats.bytes += file.size;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let on_disk = std::fs::symlink_metadata(&path).ok().filter(|m| m.is_file()).map(|m| m.len());
            let unchanged =
                old.get(file.path.as_str()).is_some_and(|o| o.size == file.size && o.sha_content == file.sha_content);
            if unchanged && !s.verify && on_disk == Some(file.size) {
                continue; // same bytes as the installed build: trust it
            }
            match on_disk {
                None => {
                    remove_any(&path);
                    let f_ = File::create(&path)?;
                    f_.set_len(file.size)?; // preallocate (sparse) so chunks can land anywhere
                    wanted.extend((0..file.chunks.len()).map(|c| (p, f, c)));
                }
                Some(len) => {
                    if len != file.size {
                        OpenOptions::new().write(true).open(&path)?.set_len(file.size)?;
                    }
                    to_check.push((p, f));
                }
            }
        }
    }
    Ok((to_check, wanted))
}

fn remove_any(path: &Path) {
    if let Ok(m) = std::fs::symlink_metadata(path) {
        let _ = if m.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) };
    }
}

/// Indexes of the chunks of `file` whose bytes on disk don't match.
fn check_file(path: &Path, file: &FileEntry, done: &AtomicU64) -> Result<Vec<usize>> {
    let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut bad = Vec::new();
    let mut buf = Vec::new();
    for (i, c) in file.chunks.iter().enumerate() {
        buf.resize(c.size as usize, 0);
        let good = read_at(&f, &mut buf, c.offset).is_ok() && Sha1::digest(&buf)[..] == c.sha;
        if !good {
            bad.push(i);
        }
        done.fetch_add(u64::from(c.size), Ordering::Relaxed);
    }
    Ok(bad)
}

fn fetch_chunk_to(s: &Settings, plan: &DepotPlan, file: &FileEntry, chunk: &Chunk) -> Result<()> {
    let data = fetch_chunk(s, plan.new.depot, &plan.key, chunk)?;
    let path = s.dir.join(&file.path);
    let f = OpenOptions::new().write(true).open(&path).with_context(|| format!("open {}", path.display()))?;
    write_at(&f, &data, chunk.offset).with_context(|| format!("write {}", path.display()))
}

/// Download, decrypt, decompress and verify one chunk, retrying with backoff.
fn fetch_chunk(s: &Settings, depot: u32, key: &DepotKey, chunk: &Chunk) -> Result<Vec<u8>> {
    let path = format!("/depot/{depot}/chunk/{}", chunk.id());
    let mut last = None;
    for attempt in 0..5u32 {
        if s.cancel.is_cancelled() {
            return Err(super::Cancelled.into());
        }
        if attempt > 0 {
            std::thread::sleep(Duration::from_millis(400 << attempt));
        }
        bandwidth::wait_while_paused(s.cancel);
        if s.cancel.is_cancelled() {
            return Err(super::Cancelled.into());
        }
        match try_chunk(s.cdn, s.cancel, &path, key, chunk) {
            Ok(d) => return Ok(d),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap()).with_context(|| format!("chunk {} of depot {depot}", chunk.id()))
}

fn try_chunk(cdn: &Cdn, cancel: &CancelToken, path: &str, key: &DepotKey, chunk: &Chunk) -> Result<Vec<u8>> {
    let body = cdn.get(path, 64 << 20)?;
    bandwidth::received(body.len() as u64, cancel);
    let plain = crypto::decrypt(key, &body)?;
    let data = compress::decompress(&plain, chunk.size as usize)?;
    ensure!(Sha1::digest(&data)[..] == chunk.sha, "chunk hash mismatch");
    ensure!(checksum(&data) == chunk.crc, "chunk checksum mismatch");
    Ok(data)
}

/// Run `work` over `items` on `s.workers` threads, calling `tick` on the caller's thread
/// while they run. The first error (or a cancel) stops everyone.
fn parallel<T: Sync>(
    items: &[T],
    s: &Settings,
    mut tick: impl FnMut(),
    work: impl Fn(&T) -> Result<()> + Sync,
) -> Result<()> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let running = AtomicUsize::new(0);
    let error: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    let workers = s.workers.clamp(1, items.len().max(1));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            running.fetch_add(1, Ordering::SeqCst);
            scope.spawn(|| {
                while !stop.load(Ordering::Relaxed) && !s.cancel.is_cancelled() {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = items.get(i) else { break };
                    if let Err(e) = work(item) {
                        error.lock().unwrap().get_or_insert(e);
                        stop.store(true, Ordering::Relaxed);
                    }
                }
                running.fetch_sub(1, Ordering::SeqCst);
            });
        }
        while running.load(Ordering::SeqCst) > 0 {
            tick();
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    tick();
    if let Some(e) = error.into_inner().unwrap() {
        return Err(e);
    }
    if s.cancel.is_cancelled() {
        return Err(super::Cancelled.into());
    }
    Ok(())
}

/// Modes, symlinks and removal of files the new build no longer has.
fn finish_files(dir: &Path, plans: &[DepotPlan], stats: &mut Stats) -> Result<()> {
    let kept: HashSet<&str> = plans.iter().flat_map(|p| &p.new.files).map(|f| f.path.as_str()).collect();
    for plan in plans {
        for file in &plan.new.files {
            let path = dir.join(&file.path);
            match &file.link {
                Some(target) => make_symlink(&path, target)?,
                None if file.is_exec() && !file.is_dir() => set_exec(&path)?,
                None => {}
            }
        }
        for gone in plan.old.iter().flat_map(|m| &m.files).filter(|f| !kept.contains(f.path.as_str())) {
            let path = dir.join(&gone.path);
            if !gone.is_dir() && std::fs::symlink_metadata(&path).is_ok() {
                remove_any(&path);
                stats.files_removed += 1;
                prune_empty_parents(dir, &path);
            }
        }
    }
    Ok(())
}

fn prune_empty_parents(root: &Path, from: &Path) {
    let mut cur = from.parent();
    while let Some(d) = cur.filter(|d| *d != root && d.starts_with(root)) {
        if std::fs::remove_dir(d).is_err() {
            break; // not empty (or gone)
        }
        cur = d.parent();
    }
}

#[cfg(unix)]
fn set_exec(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(path)?.permissions().mode();
    if mode & 0o111 != 0o111 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode | 0o755))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_exec(_: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn make_symlink(path: &Path, target: &str) -> Result<()> {
    if std::fs::read_link(path).is_ok_and(|cur| cur == Path::new(target)) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    remove_any(path);
    std::os::unix::fs::symlink(target, path).with_context(|| format!("symlink {}", path.display()))
}

#[cfg(not(unix))]
fn make_symlink(_: &Path, _: &str) -> Result<()> {
    Ok(()) // symlinks need privileges on Windows; Windows depots rarely use them
}

#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, off)
}

#[cfg(unix)]
fn read_at(f: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, off)
}

#[cfg(not(unix))]
fn write_at(f: &File, mut buf: &[u8], mut off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_write(buf, off)?;
        buf = &buf[n..];
        off += n as u64;
    }
    Ok(())
}

#[cfg(not(unix))]
fn read_at(f: &File, mut buf: &mut [u8], mut off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = f.seek_read(buf, off)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let rest = buf;
        buf = &mut rest[n..];
        off += n as u64;
    }
    Ok(())
}

/// Total size of regular files below `dir` (for `SizeOnDisk`).
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else { return 0 };
    rd.flatten()
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map_or(0, |m| m.len()),
            _ => 0,
        })
        .sum()
}
