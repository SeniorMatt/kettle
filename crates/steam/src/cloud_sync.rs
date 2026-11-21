//! Steam Cloud, the running part: ask Steam what the cloud holds, fetch newer files before a game starts and
//! send changed ones after it ended. The decisions are in [`crate::cloud`]; this file does the transfers.
//!
//! Rules that hold on every path:
//! * nothing is uploaded unless the download step before the launch succeeded (the caller only calls
//!   [`after_exit`] then) and no conflict is open;
//! * nothing is ever deleted, locally or in the cloud;
//! * a file we overwrite locally is first copied to `<data dir>/cloud-backups/<appid>/<time>/`;
//! * every step is described in a log (the launch log), and with `dry_run` nothing is changed at all.
//!
//! The protocol part ([`CmBackend`]) is written against the real message types of `steam-vent-proto-steam` but
//! has not been tried on a real account.

use crate::cloud::{
    CloudFile, Conflict, LocalFile, Mapping, PLATFORM_ALL, Plan, Resolution, SyncState, Target, local_path, plan, scan,
    sha1_hex, unix_secs,
};
use anyhow::{Context, Result, bail, ensure};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// One file to send.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub name: String,
    pub bytes: Vec<u8>,
    pub sha: String,
    pub time: u64,
    pub platforms: u32,
}

/// What the sync needs from Steam. [`CmBackend`] is the real one; tests use a fake.
pub trait Backend {
    /// The change number and the full file list.
    fn changelist(&mut self) -> Result<(u64, Vec<CloudFile>)>;
    /// The content of one file (already decompressed).
    fn download(&mut self, name: &str) -> Result<Vec<u8>>;
    /// Send files as one batch (begin, upload each, complete).
    fn upload_batch(&mut self, files: &[Outgoing]) -> Result<()>;
}

/// Everything one sync needs to know.
pub struct Job<'a> {
    pub appid: u32,
    pub maps: &'a [Mapping],
    /// `userdata/<account>/<appid>/remote`, for games that use `ISteamRemoteStorage`.
    pub remote: Option<&'a Path>,
    pub target: Target,
    /// Log what would happen and change nothing.
    pub dry_run: bool,
    pub state_path: PathBuf,
    pub backup_dir: PathBuf,
}

impl Job<'_> {
    fn scan(&self, log: &mut Vec<String>) -> Vec<LocalFile> {
        let (files, notes) = scan(self.maps, self.remote);
        log.extend(notes.into_iter().map(|n| format!("cloud: {n}")));
        files
    }

    /// The cloud list, reduced to files meant for this OS that have a place on this machine.
    fn cloud(&self, be: &mut dyn Backend, log: &mut Vec<String>) -> Result<(u64, Vec<CloudFile>)> {
        let (change, all) = be.changelist().context("reading the cloud file list")?;
        let total = all.len();
        let files: Vec<CloudFile> = all
            .into_iter()
            .filter(|f| {
                f.wanted_on(self.target) && (f.deleted || local_path(&f.name, self.maps, self.remote).is_some())
            })
            .collect();
        if files.len() != total {
            log.push(format!(
                "cloud: {} of {total} cloud files are for another system or have no known place here; left alone",
                total - files.len()
            ));
        }
        Ok((change, files))
    }

    fn load_state(&self) -> SyncState {
        crate::store::load_json(&self.state_path).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pre {
    /// Ready to start the game.
    Synced { downloaded: usize, uploaded: usize },
    /// Both sides changed; nothing was changed. Ask the user.
    Conflict { conflicts: Vec<Conflict>, local_time: u64, cloud_time: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Post {
    Uploaded(usize),
    NothingToDo,
    /// The cloud changed too while the game ran: nothing was sent.
    Conflict(usize),
}

fn describe(p: &Plan, log: &mut Vec<String>) {
    log.push(format!(
        "cloud: plan: {} to download, {} to upload, {} already equal, {} in conflict",
        p.downloads.len(),
        p.uploads.len(),
        p.in_sync.len(),
        p.conflicts.len()
    ));
    for (what, names) in [("download", &p.downloads), ("upload", &p.uploads)] {
        log.extend(names.iter().map(|n| format!("cloud:   {what} {n}")));
    }
    log.extend(
        p.conflicts
            .iter()
            .map(|c| format!("cloud:   conflict {} (local {}, cloud {})", c.name, c.local_time, c.cloud_time)),
    );
    log.extend(p.notes.iter().map(|n| format!("cloud:   {n}")));
}

/// Before the game starts. `resolution` is the user's answer to an earlier [`Pre::Conflict`].
pub fn before_launch(
    job: &Job,
    be: &mut dyn Backend,
    resolution: Option<Resolution>,
    log: &mut Vec<String>,
) -> Result<Pre> {
    let mut state = job.load_state();
    let (change, cloud) = job.cloud(be, log)?;
    let local = job.scan(log);
    let mut p = plan(&state, &local, &cloud);
    describe(&p, log);
    if !p.conflicts.is_empty() {
        match resolution {
            None => {
                let (local_time, cloud_time) = p.conflict_times();
                log.push("cloud: conflict; the game is not started until the user chooses".into());
                return Ok(Pre::Conflict { conflicts: p.conflicts, local_time, cloud_time });
            }
            Some(r) => {
                log.push(format!("cloud: conflict resolved by the user: {r:?}"));
                p.resolve(r);
            }
        }
    }
    if job.dry_run {
        log.push("cloud: dry run, nothing changed".into());
        return Ok(Pre::Synced { downloaded: 0, uploaded: 0 });
    }
    // Downloads first; any failure ends the sync with an error, and the caller then neither starts an upload
    // nor trusts the state.
    for name in &p.downloads {
        let cf = cloud.iter().find(|f| &f.name == name).context("file vanished from the list")?;
        fetch(job, be, cf, &mut state, log).with_context(|| format!("downloading {name}"))?;
    }
    // Only uploads the user chose (keep local) happen now; the rest is sent after the game.
    let chosen: Vec<&String> = p.chosen_uploads.iter().collect();
    let uploaded = send(job, be, &local, &chosen, &mut state, log)?;
    for name in &p.in_sync {
        if let Some(l) = local.iter().find(|f| &f.name == name) {
            state.record(name, &l.sha, l.time);
        }
    }
    state.change = change;
    crate::store::save_json(&job.state_path, &state)?;
    Ok(Pre::Synced { downloaded: p.downloads.len(), uploaded })
}

/// After the game ended: send what changed. Never called unless [`before_launch`] succeeded.
pub fn after_exit(job: &Job, be: &mut dyn Backend, log: &mut Vec<String>) -> Result<Post> {
    let mut state = job.load_state();
    let (change, cloud) = job.cloud(be, log)?;
    let local = job.scan(log);
    let p = plan(&state, &local, &cloud);
    describe(&p, log);
    if !p.conflicts.is_empty() {
        log.push("cloud: the cloud changed during play as well; nothing was uploaded".into());
        return Ok(Post::Conflict(p.conflicts.len()));
    }
    if job.dry_run {
        log.push("cloud: dry run, nothing changed".into());
        return Ok(Post::NothingToDo);
    }
    let names: Vec<&String> = p.uploads.iter().collect();
    let n = send(job, be, &local, &names, &mut state, log)?;
    for name in &p.in_sync {
        if let Some(l) = local.iter().find(|f| &f.name == name) {
            state.record(name, &l.sha, l.time);
        }
    }
    state.change = change;
    crate::store::save_json(&job.state_path, &state)?;
    Ok(if n == 0 { Post::NothingToDo } else { Post::Uploaded(n) })
}

/// Read the named local files and send them as one batch; record them in `state` once Steam took them.
fn send(
    job: &Job,
    be: &mut dyn Backend,
    local: &[LocalFile],
    names: &[&String],
    state: &mut SyncState,
    log: &mut Vec<String>,
) -> Result<usize> {
    if names.is_empty() {
        return Ok(0);
    }
    let mut out = Vec::new();
    for name in names {
        let l = local.iter().find(|f| &f.name == *name).context("file vanished before upload")?;
        let bytes = std::fs::read(&l.path).with_context(|| format!("reading {}", l.path.display()))?;
        // The file may have changed since the scan: send what we hold and record exactly that.
        out.push(Outgoing {
            name: l.name.clone(),
            sha: sha1_hex(&bytes),
            bytes,
            time: l.time,
            platforms: PLATFORM_ALL,
        });
    }
    be.upload_batch(&out).context("uploading to the cloud")?;
    for o in &out {
        state.record(&o.name, &o.sha, o.time);
        log.push(format!("cloud: uploaded {}", o.name));
    }
    let _ = job;
    Ok(out.len())
}

/// Download one file, check it, keep a copy of what it replaces, write it atomically and set its time.
fn fetch(job: &Job, be: &mut dyn Backend, cf: &CloudFile, state: &mut SyncState, log: &mut Vec<String>) -> Result<()> {
    let dest = local_path(&cf.name, job.maps, job.remote).context("no place for this file on this machine")?;
    let bytes = be.download(&cf.name)?;
    ensure!(cf.sha.is_empty() || sha1_hex(&bytes) == cf.sha, "the downloaded content does not match its checksum");
    if dest.is_file() {
        let stamp = unix_secs(std::time::SystemTime::now());
        let safe = cf.name.trim_start_matches('%').replace(['%', '/'], "_");
        let backup = job.backup_dir.join(stamp.to_string()).join(safe);
        if let Some(dir) = backup.parent() {
            crate::store::private_dir(dir)?;
        }
        std::fs::copy(&dest, &backup).with_context(|| format!("keeping a copy of {}", dest.display()))?;
        log.push(format!("cloud: kept a copy of the old {} in {}", cf.name, backup.display()));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = dest.with_extension("kettle-cloud-tmp");
    std::fs::write(&tmp, &bytes).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &dest)?;
    if cf.time > 0 {
        let f = std::fs::File::options().write(true).open(&dest)?;
        let _ = f.set_modified(UNIX_EPOCH + Duration::from_secs(cf.time));
    }
    state.record(&cf.name, &cf.sha, cf.time);
    log.push(format!("cloud: downloaded {} ({} bytes)", cf.name, bytes.len()));
    Ok(())
}

// ---- the real backend ----------------------------------------------------------------------------------------

/// Steam Cloud over the logged-in CM connection (`Cloud.*` service methods) and the HTTPS hosts Steam names.
pub struct CmBackend {
    pub conn: steam_vent::Connection,
    pub handle: tokio::runtime::Handle,
    pub appid: u32,
}

const CLOUD_TIMEOUT: Duration = Duration::from_secs(40);
/// Name shown to the user's other devices as the machine that wrote the files.
const MACHINE_NAME: &str = "Kettle";
const MAX_DOWNLOAD: u64 = 120 * 1024 * 1024;

fn http_headers<'a>(it: impl Iterator<Item = (&'a Option<String>, &'a Option<String>)>) -> Vec<(String, String)> {
    it.filter_map(|(n, v)| Some((n.clone()?, v.clone()?)))
        .filter(|(n, _)| !n.eq_ignore_ascii_case("host") && !n.eq_ignore_ascii_case("content-length"))
        .collect()
}

fn url_of(https: Option<bool>, host: &str, path: &str) -> String {
    format!("{}://{host}{path}", if https.unwrap_or(true) { "https" } else { "http" })
}

/// Steam stores a file smaller than it is as a one-entry zip archive.
pub fn unzip_single(data: &[u8]) -> Result<Vec<u8>> {
    use std::io::Read;
    let u16at = |i: usize| data.get(i..i + 2).map(|b| usize::from(u16::from_le_bytes([b[0], b[1]])));
    let u32at = |i: usize| data.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize);
    ensure!(data.starts_with(b"PK\x03\x04"), "not a zip archive");
    let (method, name_len, extra_len, comp) = (
        u16at(8).context("short zip")?,
        u16at(26).context("short zip")?,
        u16at(28).context("short zip")?,
        u32at(18).context("short zip")?,
    );
    let body = data.get(30 + name_len + extra_len..).context("short zip")?;
    let mut out = Vec::new();
    match method {
        0 => out.extend_from_slice(body.get(..comp).context("short zip")?),
        8 => {
            flate2::read::DeflateDecoder::new(body).take(MAX_DOWNLOAD).read_to_end(&mut out).context("inflating")?;
        }
        m => bail!("unsupported zip method {m}"),
    }
    Ok(out)
}

impl CmBackend {
    fn call<R>(&self, f: impl std::future::Future<Output = Result<R, steam_vent::NetworkError>>) -> Result<R> {
        Ok(self.handle.block_on(f)?)
    }

    fn conn(&self) -> steam_vent::Connection {
        let mut c = self.conn.clone();
        c.set_timeout(CLOUD_TIMEOUT);
        c
    }

    fn put_block(
        &self,
        b: &steam_vent_proto_steam::steammessages_cloud_steamclient::ClientCloudFileUploadBlockDetails,
        bytes: &[u8],
    ) -> Result<()> {
        let (host, path) =
            (b.url_host.as_deref().context("upload block without host")?, b.url_path.as_deref().unwrap_or("/"));
        let url = url_of(b.use_https, host, path);
        let body: &[u8] = match &b.explicit_body_data {
            Some(d) if !d.is_empty() => d,
            _ => {
                let off = b.block_offset.unwrap_or(0) as usize;
                let len = b.block_length.map_or(bytes.len().saturating_sub(off), |l| l as usize);
                bytes.get(off..off.checked_add(len).context("bad block")?).context("upload block outside the file")?
            }
        };
        let agent = crate::http_dl::agent();
        let headers = http_headers(b.request_headers.iter().map(|h| (&h.name, &h.value)));
        macro_rules! with_headers {
            ($req:expr) => {{
                let mut r = $req;
                for (k, v) in &headers {
                    r = r.header(k.as_str(), v.as_str());
                }
                r.send(body)
            }};
        }
        let res = match b.http_method.unwrap_or(4) {
            3 => with_headers!(agent.post(&url)),
            _ => with_headers!(agent.put(&url)),
        };
        res.with_context(|| format!("sending a file part to {host}"))?;
        Ok(())
    }
}

impl Backend for CmBackend {
    fn changelist(&mut self) -> Result<(u64, Vec<CloudFile>)> {
        use steam_vent::ConnectionTrait;
        use steam_vent_proto_steam::steammessages_cloud_steamclient::CCloud_GetAppFileChangelist_Request;
        let req = CCloud_GetAppFileChangelist_Request {
            appid: Some(self.appid),
            synced_change_number: Some(0),
            ..Default::default()
        };
        let conn = self.conn();
        let resp = self.call(conn.service_method(req))?;
        let files = resp
            .files
            .iter()
            .filter_map(|f| {
                let file_name = f.file_name.as_deref()?;
                // The name is `<prefix><file_name>`; a name that already carries a `%Root%` stays as it is.
                let prefix =
                    f.path_prefix_index.and_then(|i| resp.path_prefixes.get(i as usize)).map_or("", String::as_str);
                let name =
                    if file_name.starts_with('%') { file_name.to_string() } else { format!("{prefix}{file_name}") };
                let state = f.persist_state.map_or(0, |s| s.value());
                Some(CloudFile {
                    name,
                    sha: crate::cloud::hex(f.sha_file.as_deref().unwrap_or_default()),
                    time: f.time_stamp.unwrap_or(0),
                    size: f.raw_file_size.unwrap_or(0),
                    // 0 persisted, 1 forgotten, 2 deleted
                    deleted: state != 0,
                    platforms: f.platforms_to_sync.unwrap_or(PLATFORM_ALL),
                })
            })
            .collect();
        Ok((resp.current_change_number.unwrap_or(0), files))
    }

    fn download(&mut self, name: &str) -> Result<Vec<u8>> {
        use steam_vent::ConnectionTrait;
        use steam_vent_proto_steam::steammessages_cloud_steamclient::CCloud_ClientFileDownload_Request;
        let req = CCloud_ClientFileDownload_Request {
            appid: Some(self.appid),
            filename: Some(name.to_string()),
            realm: Some(1),
            ..Default::default()
        };
        let conn = self.conn();
        let r = self.call(conn.service_method(req))?;
        ensure!(r.encrypted != Some(true), "the file is encrypted by Steam, which is not supported");
        let host = r.url_host.as_deref().context("no download address")?;
        let url = url_of(r.use_https, host, r.url_path.as_deref().unwrap_or("/"));
        let headers = http_headers(r.request_headers.iter().map(|h| (&h.name, &h.value)));
        let mut req = crate::http_dl::agent().get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let raw = req
            .call()
            .with_context(|| format!("fetching from {host}"))?
            .into_body()
            .with_config()
            .limit(MAX_DOWNLOAD)
            .read_to_vec()?;
        let compressed = r.raw_file_size.is_some() && r.file_size != r.raw_file_size;
        if compressed || (raw.starts_with(b"PK\x03\x04") && r.raw_file_size.is_some_and(|n| n as usize != raw.len())) {
            unzip_single(&raw)
        } else {
            Ok(raw)
        }
    }

    fn upload_batch(&mut self, files: &[Outgoing]) -> Result<()> {
        use steam_vent::ConnectionTrait;
        use steam_vent_proto_steam::steammessages_cloud_steamclient::{
            CCloud_BeginAppUploadBatch_Request, CCloud_ClientBeginFileUpload_Request,
            CCloud_ClientCommitFileUpload_Request, CCloud_CompleteAppUploadBatch_Request,
        };
        let conn = self.conn();
        let batch = self
            .call(conn.service_method(CCloud_BeginAppUploadBatch_Request {
                appid: Some(self.appid),
                machine_name: Some(MACHINE_NAME.into()),
                files_to_upload: files.iter().map(|f| f.name.clone()).collect(),
                ..Default::default()
            }))?
            .batch_id
            .context("Steam gave no upload batch")?;
        let result = (|| -> Result<()> {
            for f in files {
                let sha = (0..f.sha.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&f.sha[i..i + 2], 16))
                    .collect::<Result<Vec<u8>, _>>()?;
                let size = u32::try_from(f.bytes.len()).context("file too large")?;
                let begin = self.call(conn.service_method(CCloud_ClientBeginFileUpload_Request {
                    appid: Some(self.appid),
                    file_size: Some(size),
                    raw_file_size: Some(size),
                    file_sha: Some(sha.clone()),
                    time_stamp: Some(f.time),
                    filename: Some(f.name.clone()),
                    platforms_to_sync: Some(f.platforms),
                    can_encrypt: Some(false),
                    upload_batch_id: Some(batch),
                    ..Default::default()
                }))?;
                ensure!(
                    begin.encrypt_file != Some(true),
                    "Steam asked for an encrypted upload, which is not supported"
                );
                let sent: Result<()> = begin.block_requests.iter().try_for_each(|b| self.put_block(b, &f.bytes));
                let ok = sent.is_ok();
                let commit = self.call(conn.service_method(CCloud_ClientCommitFileUpload_Request {
                    transfer_succeeded: Some(ok),
                    appid: Some(self.appid),
                    file_sha: Some(sha),
                    filename: Some(f.name.clone()),
                    ..Default::default()
                }));
                sent?;
                // Nothing to send means Steam already holds this content; a refused commit then is harmless.
                if !begin.block_requests.is_empty() {
                    ensure!(commit?.file_committed == Some(true), "Steam did not accept {}", f.name);
                }
            }
            Ok(())
        })();
        let done = self.call(conn.service_method(CCloud_CompleteAppUploadBatch_Request {
            appid: Some(self.appid),
            batch_id: Some(batch),
            batch_eresult: Some(if result.is_ok() { 1 } else { 2 }),
            ..Default::default()
        }));
        result?;
        done.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Fake {
        cloud: BTreeMap<String, (Vec<u8>, u64)>,
        fail_download: bool,
        uploads: Vec<Vec<String>>,
        downloads: Vec<String>,
    }

    impl Backend for Fake {
        fn changelist(&mut self) -> Result<(u64, Vec<CloudFile>)> {
            let files = self
                .cloud
                .iter()
                .map(|(n, (b, t))| CloudFile {
                    name: n.clone(),
                    sha: sha1_hex(b),
                    time: *t,
                    size: b.len() as u32,
                    deleted: false,
                    platforms: PLATFORM_ALL,
                })
                .collect();
            Ok((7, files))
        }

        fn download(&mut self, name: &str) -> Result<Vec<u8>> {
            self.downloads.push(name.to_string());
            ensure!(!self.fail_download, "network down");
            Ok(self.cloud[name].0.clone())
        }

        fn upload_batch(&mut self, files: &[Outgoing]) -> Result<()> {
            self.uploads.push(files.iter().map(|f| f.name.clone()).collect());
            for f in files {
                self.cloud.insert(f.name.clone(), (f.bytes.clone(), f.time));
            }
            Ok(())
        }
    }

    struct Dirs(PathBuf);

    impl Dirs {
        fn new(tag: &str) -> Dirs {
            let d = std::env::temp_dir().join(format!("sr-cloudsync-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(d.join("saves")).unwrap();
            Dirs(d)
        }

        fn maps(&self) -> Vec<Mapping> {
            vec![Mapping {
                token: "%GameInstall%saves/".into(),
                dir: self.0.join("saves"),
                patterns: vec!["*.sav".into()],
                recursive: false,
            }]
        }

        fn job<'a>(&'a self, maps: &'a [Mapping], dry_run: bool) -> Job<'a> {
            Job {
                appid: 5,
                maps,
                remote: None,
                target: Target::Linux,
                dry_run,
                state_path: self.0.join("state.json"),
                backup_dir: self.0.join("backups"),
            }
        }

        fn read(&self, f: &str) -> String {
            std::fs::read_to_string(self.0.join("saves").join(f)).unwrap_or_default()
        }
    }

    impl Drop for Dirs {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const A: &str = "%GameInstall%saves/a.sav";

    #[test]
    fn downloads_newer_cloud_files_and_remembers_them() {
        let d = Dirs::new("down");
        let maps = d.maps();
        let mut be = Fake::default();
        be.cloud.insert(A.into(), (b"cloud".to_vec(), 1_700_000_000));
        let mut log = vec![];
        let r = before_launch(&d.job(&maps, false), &mut be, None, &mut log).unwrap();
        assert_eq!(r, Pre::Synced { downloaded: 1, uploaded: 0 });
        assert_eq!(d.read("a.sav"), "cloud");
        let modified = std::fs::metadata(d.0.join("saves/a.sav")).unwrap().modified().unwrap();
        assert_eq!(unix_secs(modified), 1_700_000_000);
        let state: SyncState = crate::store::load_json(&d.0.join("state.json")).unwrap();
        assert_eq!((state.change, state.files[A].sha.clone()), (7, sha1_hex(b"cloud")));
        // Second launch: equal on both sides, nothing to do.
        let r = before_launch(&d.job(&maps, false), &mut be, None, &mut vec![]).unwrap();
        assert_eq!(r, Pre::Synced { downloaded: 0, uploaded: 0 });
        assert_eq!(be.downloads.len(), 1);
    }

    #[test]
    fn failed_download_is_an_error_and_nothing_is_uploaded() {
        let d = Dirs::new("fail");
        let maps = d.maps();
        std::fs::write(d.0.join("saves/b.sav"), "local only").unwrap();
        let mut be = Fake { fail_download: true, ..Default::default() };
        be.cloud.insert(A.into(), (b"cloud".to_vec(), 5));
        let r = before_launch(&d.job(&maps, false), &mut be, None, &mut vec![]);
        assert!(r.is_err());
        assert!(be.uploads.is_empty());
        assert!(!d.0.join("state.json").exists(), "state must not claim a sync that failed");
    }

    #[test]
    fn upload_after_exit_sends_only_changes() {
        let d = Dirs::new("up");
        let maps = d.maps();
        let mut be = Fake::default();
        be.cloud.insert(A.into(), (b"v1".to_vec(), 10));
        before_launch(&d.job(&maps, false), &mut be, None, &mut vec![]).unwrap();
        // Nothing changed while playing.
        assert_eq!(after_exit(&d.job(&maps, false), &mut be, &mut vec![]).unwrap(), Post::NothingToDo);
        assert!(be.uploads.is_empty());
        // The game wrote a save and a new one.
        std::fs::write(d.0.join("saves/a.sav"), "v2").unwrap();
        std::fs::write(d.0.join("saves/n.sav"), "new").unwrap();
        let r = after_exit(&d.job(&maps, false), &mut be, &mut vec![]).unwrap();
        assert_eq!(r, Post::Uploaded(2));
        assert_eq!(be.uploads, [vec![A.to_string(), "%GameInstall%saves/n.sav".to_string()]]);
        assert_eq!(after_exit(&d.job(&maps, false), &mut be, &mut vec![]).unwrap(), Post::NothingToDo);
        assert_eq!(be.uploads.len(), 1);
    }

    #[test]
    fn conflict_asks_first_and_changes_nothing() {
        let d = Dirs::new("conf");
        let maps = d.maps();
        std::fs::write(d.0.join("saves/a.sav"), "local").unwrap();
        let mut be = Fake::default();
        be.cloud.insert(A.into(), (b"cloud".to_vec(), 5));
        let mut log = vec![];
        let r = before_launch(&d.job(&maps, false), &mut be, None, &mut log).unwrap();
        let Pre::Conflict { conflicts, cloud_time, .. } = r else { panic!("expected a conflict") };
        assert_eq!((conflicts.len(), cloud_time), (1, 5));
        assert_eq!(d.read("a.sav"), "local");
        assert!(be.downloads.is_empty() && be.uploads.is_empty());
        assert!(!d.0.join("state.json").exists());
        assert!(log.iter().any(|l| l.contains("conflict")));
    }

    #[test]
    fn keep_cloud_overwrites_and_backs_up() {
        let d = Dirs::new("kc");
        let maps = d.maps();
        std::fs::write(d.0.join("saves/a.sav"), "local").unwrap();
        let mut be = Fake::default();
        be.cloud.insert(A.into(), (b"cloud".to_vec(), 5));
        let r = before_launch(&d.job(&maps, false), &mut be, Some(Resolution::KeepCloud), &mut vec![]).unwrap();
        assert_eq!(r, Pre::Synced { downloaded: 1, uploaded: 0 });
        assert_eq!(d.read("a.sav"), "cloud");
        let kept: Vec<_> = walk_files(&d.0.join("backups"));
        assert_eq!(kept.len(), 1);
        assert_eq!(std::fs::read_to_string(&kept[0]).unwrap(), "local");
    }

    fn walk_files(dir: &Path) -> Vec<PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() { out.extend(walk_files(&p)) } else { out.push(p) }
        }
        out
    }

    #[test]
    fn keep_local_uploads_before_launch() {
        let d = Dirs::new("kl");
        let maps = d.maps();
        std::fs::write(d.0.join("saves/a.sav"), "local").unwrap();
        let mut be = Fake::default();
        be.cloud.insert(A.into(), (b"cloud".to_vec(), 5));
        let r = before_launch(&d.job(&maps, false), &mut be, Some(Resolution::KeepLocal), &mut vec![]).unwrap();
        assert_eq!(r, Pre::Synced { downloaded: 0, uploaded: 1 });
        assert_eq!(be.cloud[A].0, b"local");
        assert_eq!(d.read("a.sav"), "local");
    }

    #[test]
    fn conflict_after_play_blocks_the_upload() {
        let d = Dirs::new("post");
        let maps = d.maps();
        let mut be = Fake::default();
        be.cloud.insert(A.into(), (b"v1".to_vec(), 5));
        before_launch(&d.job(&maps, false), &mut be, None, &mut vec![]).unwrap();
        std::fs::write(d.0.join("saves/a.sav"), "mine").unwrap();
        be.cloud.insert(A.into(), (b"theirs".to_vec(), 9));
        assert_eq!(after_exit(&d.job(&maps, false), &mut be, &mut vec![]).unwrap(), Post::Conflict(1));
        assert!(be.uploads.is_empty());
        assert_eq!(be.cloud[A].0, b"theirs");
    }

    #[test]
    fn dry_run_changes_nothing() {
        let d = Dirs::new("dry");
        let maps = d.maps();
        let mut be = Fake::default();
        be.cloud.insert(A.into(), (b"cloud".to_vec(), 5));
        let mut log = vec![];
        before_launch(&d.job(&maps, true), &mut be, None, &mut log).unwrap();
        assert!(!d.0.join("saves/a.sav").exists() && !d.0.join("state.json").exists());
        assert!(log.iter().any(|l| l.contains("download %GameInstall%saves/a.sav")));
        std::fs::write(d.0.join("saves/z.sav"), "z").unwrap();
        after_exit(&d.job(&maps, true), &mut be, &mut log).unwrap();
        assert!(be.uploads.is_empty());
    }

    #[test]
    fn corrupt_download_is_refused() {
        let d = Dirs::new("bad");
        let maps = d.maps();
        struct Liar(Fake);
        impl Backend for Liar {
            fn changelist(&mut self) -> Result<(u64, Vec<CloudFile>)> {
                self.0.changelist()
            }
            fn download(&mut self, _: &str) -> Result<Vec<u8>> {
                Ok(b"something else".to_vec())
            }
            fn upload_batch(&mut self, f: &[Outgoing]) -> Result<()> {
                self.0.upload_batch(f)
            }
        }
        let mut be = Liar(Fake::default());
        be.0.cloud.insert(A.into(), (b"cloud".to_vec(), 5));
        assert!(before_launch(&d.job(&maps, false), &mut be, None, &mut vec![]).is_err());
        assert!(!d.0.join("saves/a.sav").exists());
    }

    #[test]
    fn unknown_cloud_names_are_left_alone() {
        let d = Dirs::new("unk");
        let maps = d.maps();
        let mut be = Fake::default();
        be.cloud.insert("%WinMyDocuments%elsewhere/x.sav".into(), (b"x".to_vec(), 5));
        be.cloud.insert("../escape".into(), (b"x".to_vec(), 5));
        let r = before_launch(&d.job(&maps, false), &mut be, None, &mut vec![]).unwrap();
        assert_eq!(r, Pre::Synced { downloaded: 0, uploaded: 0 });
        assert!(be.downloads.is_empty());
    }

    #[test]
    fn single_entry_zip_is_unpacked() {
        use std::io::Write;
        let payload = b"hello hello hello hello save data";
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(payload).unwrap();
        let comp = enc.finish().unwrap();
        let mut z = b"PK\x03\x04".to_vec();
        z.extend_from_slice(&[20, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0]); // version, flags, method 8, time, date, crc
        z.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        z.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        z.extend_from_slice(&[1, 0, 0, 0]); // name length 1, no extra
        z.push(b'f');
        z.extend_from_slice(&comp);
        assert_eq!(unzip_single(&z).unwrap(), payload);
        assert!(unzip_single(b"plain").is_err());
    }
}
