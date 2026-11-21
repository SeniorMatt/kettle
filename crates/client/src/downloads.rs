//! The download queue: one active download at a time, the rest wait.
//!
//! The downloader (`sr_steam::content::Download`) runs on its own thread. A small pump thread per download
//! forwards its progress to the reducer as events tagged with the job number (the generation), at most four
//! times a second, and wakes the UI each time. When nothing downloads, nothing here runs, so idle CPU stays 0.
//!
//! Pause is cancel plus resume later: the downloader resumes by hashing what is already on disk.

use super::{Client, Event, Waker};
use sr_model::AppId;
use sr_steam::content::{self, Auth, Download, Options, Os, Progress};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub use content::Phase as DownloadPhase;

/// Progress is forwarded to the UI at most this often (about 4 Hz).
const FORWARD_EVERY: Duration = Duration::from_millis(250);
/// How often the pump looks at the pause/cancel flag.
const POLL: Duration = Duration::from_millis(100);
/// Bounds, so a runaway UI can't grow the lists.
const MAX_QUEUE: usize = 50;
const MAX_FINISHED: usize = 20;

/// Target OS of a download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OsChoice {
    #[default]
    Auto,
    Linux,
    Windows,
}

impl OsChoice {
    pub fn label(self) -> &'static str {
        match self {
            OsChoice::Auto => "Automatic",
            OsChoice::Linux => "Linux",
            OsChoice::Windows => "Windows",
        }
    }

    fn os(self) -> Option<Os> {
        match self {
            OsChoice::Auto => None,
            OsChoice::Linux => Some(Os::Linux),
            OsChoice::Windows => Some(Os::Windows),
        }
    }
}

/// What the install dialog asks for.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadRequest {
    pub app: AppId,
    pub name: String,
    /// Library root (Steam format: files go to `<library>/steamapps/common/<installdir>`).
    pub library: PathBuf,
    pub os: OsChoice,
    pub verify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadKind {
    Install,
    Update,
    Verify,
}

impl DownloadKind {
    pub fn label(self) -> &'static str {
        match self {
            DownloadKind::Install => "Install",
            DownloadKind::Update => "Update",
            DownloadKind::Verify => "Verify",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DownloadItem {
    pub request: DownloadRequest,
    pub kind: DownloadKind,
}

/// Why the active download is being stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopKind {
    Pause,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct ActiveDownload {
    pub item: DownloadItem,
    pub phase: DownloadPhase,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_sec: f64,
    /// Latest warning from the downloader.
    pub message: Option<String>,
    /// A pause or cancel was requested and the downloader is winding down.
    pub stopping: Option<StopKind>,
}

impl ActiveDownload {
    pub fn fraction(&self) -> f32 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.done_bytes as f64 / self.total_bytes as f64).clamp(0.0, 1.0) as f32
        }
    }

    /// Seconds left at the current speed, when it can be told.
    pub fn eta_secs(&self) -> Option<u64> {
        let left = self.total_bytes.saturating_sub(self.done_bytes);
        (self.bytes_per_sec > 1.0 && left > 0).then(|| (left as f64 / self.bytes_per_sec) as u64)
    }
}

#[derive(Debug, Clone)]
pub struct PausedDownload {
    pub item: DownloadItem,
    pub done_bytes: u64,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Done { bytes_downloaded: u64, seconds: f64, up_to_date: bool },
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct FinishedDownload {
    pub item: DownloadItem,
    pub outcome: Outcome,
    /// Unix seconds.
    pub at: u64,
}

#[derive(Debug, Clone, Default)]
pub struct DownloadsState {
    pub active: Option<ActiveDownload>,
    pub queue: Vec<DownloadItem>,
    pub paused: Vec<PausedDownload>,
    /// Newest first.
    pub finished: VecDeque<FinishedDownload>,
}

/// How a game relates to the queue, for its page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DownloadStatus {
    None,
    Queued,
    Paused,
    /// Fraction 0..1.
    Active(f32),
}

impl DownloadsState {
    pub fn status(&self, app: AppId) -> DownloadStatus {
        if let Some(a) = self.active.as_ref().filter(|a| a.item.request.app == app) {
            return DownloadStatus::Active(a.fraction());
        }
        if self.queue.iter().any(|i| i.request.app == app) {
            return DownloadStatus::Queued;
        }
        if self.paused.iter().any(|p| p.item.request.app == app) {
            return DownloadStatus::Paused;
        }
        DownloadStatus::None
    }

    /// Anything downloading, queued or paused (the Downloads button shows a dot).
    pub fn is_busy(&self) -> bool {
        self.active.is_some() || !self.queue.is_empty()
    }
}

/// What the pump reports when the downloader thread has ended.
#[derive(Debug)]
pub enum DlResult {
    Done { bytes_downloaded: u64, seconds: f64, up_to_date: bool, warnings: Vec<String> },
    Failed(String),
    Cancelled,
}

/// The running job, kept by the client. `ctl` is 0 = go on, 1 = pause, 2 = cancel.
pub(crate) struct RunningDl {
    pub job: u64,
    pub(crate) ctl: Arc<AtomicU8>,
}

impl Client {
    /// Add a download to the queue and start it when nothing else runs.
    pub(crate) fn dl_enqueue(&mut self, req: DownloadRequest) {
        let app = req.app;
        if self.state.downloads.status(app) != DownloadStatus::None {
            self.state.notice = Some(format!("{} is already in the download queue.", req.name));
            return;
        }
        if self.state.downloads.queue.len() >= MAX_QUEUE {
            self.state.notice = Some("The download queue is full.".into());
            return;
        }
        let installed = self.state.game(app).is_some_and(|g| g.install.is_installed());
        let kind = match (req.verify, installed) {
            (true, _) => DownloadKind::Verify,
            (false, true) => DownloadKind::Update,
            (false, false) => DownloadKind::Install,
        };
        self.state.downloads.queue.push(DownloadItem { request: req, kind });
        self.dl_start_next();
    }

    /// Queue a verify of an installed game, in the library it lives in.
    pub(crate) fn dl_verify(&mut self, app: AppId) {
        let Some(a) = self.installed.get(&app.0) else {
            self.state.notice = Some("This game is not installed.".into());
            return;
        };
        let Some(library) = library_of(&a.install_dir) else {
            self.state.notice = Some("Can't tell which library this game is in.".into());
            return;
        };
        let name = a.name.clone();
        self.dl_enqueue(DownloadRequest { app, name, library, os: OsChoice::Auto, verify: true });
    }

    /// Queue an update of an installed game, in its own library.
    pub(crate) fn dl_update(&mut self, app: AppId) {
        let Some(a) = self.installed.get(&app.0) else { return };
        let Some(library) = library_of(&a.install_dir) else { return };
        let name = a.name.clone();
        self.dl_enqueue(DownloadRequest { app, name, library, os: OsChoice::Auto, verify: false });
    }

    pub(crate) fn dl_pause(&mut self, app: AppId) {
        self.dl_stop_active(app, StopKind::Pause);
    }

    pub(crate) fn dl_cancel(&mut self, app: AppId) {
        let d = &mut self.state.downloads;
        d.queue.retain(|i| i.request.app != app);
        d.paused.retain(|p| p.item.request.app != app);
        self.dl_stop_active(app, StopKind::Cancel);
    }

    pub(crate) fn dl_resume(&mut self, app: AppId) {
        let d = &mut self.state.downloads;
        if let Some(i) = d.paused.iter().position(|p| p.item.request.app == app) {
            let p = d.paused.remove(i);
            d.queue.insert(0, p.item);
            self.dl_start_next();
        }
    }

    pub(crate) fn dl_clear_finished(&mut self) {
        self.state.downloads.finished.clear();
    }

    /// Cancel everything (sign-out: the account that asked for them is gone).
    pub(crate) fn dl_cancel_all(&mut self) {
        let d = &mut self.state.downloads;
        d.queue.clear();
        d.paused.clear();
        if let Some(app) = d.active.as_ref().map(|a| a.item.request.app) {
            self.dl_stop_active(app, StopKind::Cancel);
        }
    }

    fn dl_stop_active(&mut self, app: AppId, why: StopKind) {
        let (Some(run), Some(a)) = (&self.dl_running, &mut self.state.downloads.active) else { return };
        if a.item.request.app != app {
            return;
        }
        a.stopping = Some(why);
        run.ctl.store(if why == StopKind::Pause { 1 } else { 2 }, Ordering::SeqCst);
    }

    fn dl_start_next(&mut self) {
        if self.dl_running.is_some() || self.state.downloads.queue.is_empty() {
            return;
        }
        let item = self.state.downloads.queue.remove(0);
        self.dl_seq += 1;
        let job = self.dl_seq;
        let r = &item.request;
        let mut opts = Options::new(r.app.0, r.library.clone());
        opts.os = r.os.os();
        opts.verify = r.verify;
        opts.workers = self.state.settings.workers;
        // Signed in: use the shared connection (a second logon of the same account would replace it).
        let hub = self.session.as_ref().and_then(|_| self.hub.clone());
        let owner = self.session.as_ref().map_or(0, |s| s.account.steam_id.0);
        let ctl = Arc::new(AtomicU8::new(0));
        let (tx, wake, ctl2, library) = (self.tx.clone(), self.wake.clone(), ctl.clone(), r.library.clone());
        let spawned =
            std::thread::Builder::new().name(format!("download-{}", r.app)).spawn(move || {
                match wait_for_auth(hub, owner, &ctl2) {
                    Ok(auth) => pump(Download::start(opts, auth), job, ctl2, &library, tx, wake),
                    Err(result) => {
                        let _ = tx.blocking_send(Event::DlDone { job, result });
                        wake();
                    }
                }
            });
        if let Err(e) = spawned {
            self.state.notice = Some(format!("Couldn't start the download: {e}"));
            self.finish(item, Outcome::Failed(e.to_string()));
            return;
        }
        self.dl_running = Some(RunningDl { job, ctl });
        self.state.downloads.active = Some(ActiveDownload {
            item,
            phase: DownloadPhase::Preparing,
            done_bytes: 0,
            total_bytes: 0,
            bytes_per_sec: 0.0,
            message: None,
            stopping: None,
        });
    }

    pub(crate) fn dl_progress(&mut self, job: u64, p: Progress) {
        if self.dl_running.as_ref().map(|r| r.job) != Some(job) {
            return; // stale: a job that already ended
        }
        if let Some(a) = &mut self.state.downloads.active {
            (a.phase, a.done_bytes, a.total_bytes, a.bytes_per_sec) =
                (p.phase, p.done_bytes, p.total_bytes, p.bytes_per_sec);
            if p.message.is_some() {
                a.message = p.message;
            }
        }
    }

    pub(crate) fn dl_done(&mut self, job: u64, result: DlResult) {
        if self.dl_running.as_ref().map(|r| r.job) != Some(job) {
            return;
        }
        let intent = self.dl_running.take().map_or(0, |r| r.ctl.load(Ordering::SeqCst));
        let Some(active) = self.state.downloads.active.take() else { return };
        let item = active.item.clone();
        let name = item.request.name.clone();
        match (result, intent) {
            (DlResult::Done { bytes_downloaded, seconds, up_to_date, warnings }, _) => {
                if let Some(w) = warnings.last() {
                    self.state.notice = Some(format!("{name}: {w}"));
                }
                self.finish(item, Outcome::Done { bytes_downloaded, seconds, up_to_date });
            }
            (DlResult::Cancelled, 1) => {
                let paused = PausedDownload { item, done_bytes: active.done_bytes, total_bytes: active.total_bytes };
                self.state.downloads.paused.push(paused);
            }
            (DlResult::Cancelled, _) => self.finish(item, Outcome::Cancelled),
            (DlResult::Failed(e), _) => {
                self.state.notice = Some(format!("{name}: {e}"));
                self.finish(item, Outcome::Failed(e));
            }
        }
        self.dl_start_next();
        // Whatever happened, the files on disk may have changed.
        self.spawn_local_scan();
    }

    fn finish(&mut self, item: DownloadItem, outcome: Outcome) {
        let f = &mut self.state.downloads.finished;
        f.push_front(FinishedDownload { item, outcome, at: now_unix() });
        f.truncate(MAX_FINISHED);
    }
}

/// `<library>/steamapps/common/<dir>` -> `<library>`.
pub(crate) fn library_of(install_dir: &Path) -> Option<PathBuf> {
    let common = install_dir.parent()?;
    let steamapps = common.parent()?;
    (common.file_name()? == "common" && steamapps.file_name()? == "steamapps")
        .then(|| steamapps.parent().map(Path::to_path_buf))?
}

pub(crate) fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Forward progress of one download until its thread ends, then report how it ended.
/// How long a download waits for the shared Steam connection before it fails.
const HUB_WAIT: Duration = Duration::from_secs(90);

/// The way a download logs in: anonymous without an account, otherwise the shared connection (waiting for
/// it while it connects). Stops early when the download is paused or cancelled meanwhile.
fn wait_for_auth(hub: Option<crate::cm_hub::HubHandle>, owner: u64, ctl: &AtomicU8) -> Result<Auth, DlResult> {
    let Some(hub) = hub else { return Ok(Auth::Anonymous) };
    let start = Instant::now();
    loop {
        if ctl.load(Ordering::SeqCst) != 0 {
            return Err(DlResult::Cancelled);
        }
        if let Some(cm) = hub.online_now() {
            return Ok(Auth::Connection { conn: cm.conn.clone(), owner });
        }
        if start.elapsed() > HUB_WAIT {
            return Err(DlResult::Failed("Not connected to Steam yet. Try again in a moment.".into()));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn pump(dl: Download, job: u64, ctl: Arc<AtomicU8>, library: &Path, tx: mpsc::Sender<Event>, wake: Waker) {
    let mut latest: Option<Progress> = None;
    let mut last_sent = Instant::now().checked_sub(FORWARD_EVERY).unwrap_or_else(Instant::now);
    let mut low_space: Option<String> = None;
    loop {
        match dl.progress.recv_timeout(POLL) {
            Ok(p) => latest = Some(p),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(p) = dl.progress.try_recv() {
            latest = Some(p);
        }
        if ctl.load(Ordering::SeqCst) != 0 {
            dl.cancel();
        }
        if low_space.is_none() {
            low_space = latest.as_ref().and_then(|p| not_enough_space(p, library));
            if low_space.is_some() {
                dl.cancel();
            }
        }
        if latest.is_some() && last_sent.elapsed() >= FORWARD_EVERY {
            let _ = tx.blocking_send(Event::DlProgress { job, progress: latest.take().expect("checked") });
            wake();
            last_sent = Instant::now();
        }
    }
    let result = match (dl.join(), low_space) {
        (_, Some(msg)) => DlResult::Failed(msg),
        (Ok(s), None) => DlResult::Done {
            bytes_downloaded: s.stats.bytes_downloaded,
            seconds: s.seconds,
            up_to_date: s.up_to_date,
            warnings: s.warnings,
        },
        (Err(e), None) if content::is_cancelled(&e) => DlResult::Cancelled,
        (Err(e), None) => DlResult::Failed(format!("{e:#}")),
    };
    let _ = tx.blocking_send(Event::DlDone { job, result });
    wake();
}

/// The first time the downloader knows how much it has to fetch, compare with the free space.
fn not_enough_space(p: &Progress, library: &Path) -> Option<String> {
    if p.phase != DownloadPhase::Downloading || p.total_bytes == 0 {
        return None;
    }
    let free = free_space(library)?;
    (p.total_bytes > free)
        .then(|| format!("Not enough disk space: {} needed, {} free.", human(p.total_bytes), human(free)))
}

fn human(b: u64) -> String {
    format!("{:.1} GB", b as f64 / (1u64 << 30) as f64)
}

/// Free bytes on the disk holding `path` (or its nearest existing parent). `None` when unknown.
pub fn free_space(path: &Path) -> Option<u64> {
    let mut dir = path;
    while !dir.exists() {
        dir = dir.parent()?;
    }
    platform_free_space(dir)
}

// The statvfs fields are u64 on Linux but narrower on some other Unixes, so the casts are needed there.
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn platform_free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut s = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `c` is a valid NUL-terminated path and `s` is a writable statvfs; we read it only on success.
    let ok = unsafe { libc::statvfs(c.as_ptr(), s.as_mut_ptr()) } == 0;
    // SAFETY: statvfs succeeded, so the struct is initialised.
    ok.then(|| unsafe { s.assume_init() }).map(|s| (s.f_bavail as u64).saturating_mul(s.f_frsize as u64))
}

/// No cheap std way on Windows; the dialog then shows "unknown".
#[cfg(not(unix))]
fn platform_free_space(_dir: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn library_from_install_dir() {
        let p = Path::new("/lib/steamapps/common/Game");
        assert_eq!(library_of(p), Some(PathBuf::from("/lib")));
        assert_eq!(library_of(Path::new("/games/Game")), None);
    }

    #[test]
    fn status_and_eta() {
        let mut d = DownloadsState::default();
        assert_eq!(d.status(AppId(1)), DownloadStatus::None);
        let item = DownloadItem {
            request: DownloadRequest {
                app: AppId(1),
                name: "G".into(),
                library: "/l".into(),
                os: OsChoice::Auto,
                verify: false,
            },
            kind: DownloadKind::Install,
        };
        d.queue.push(item.clone());
        assert_eq!(d.status(AppId(1)), DownloadStatus::Queued);
        d.queue.clear();
        d.active = Some(ActiveDownload {
            item,
            phase: DownloadPhase::Downloading,
            done_bytes: 25,
            total_bytes: 100,
            bytes_per_sec: 5.0,
            message: None,
            stopping: None,
        });
        assert_eq!(d.status(AppId(1)), DownloadStatus::Active(0.25));
        assert_eq!(d.active.as_ref().and_then(ActiveDownload::eta_secs), Some(15));
    }

    #[cfg(unix)]
    #[test]
    fn free_space_of_tmp_is_known() {
        assert!(free_space(Path::new("/tmp/does/not/exist")).is_some());
    }
}
