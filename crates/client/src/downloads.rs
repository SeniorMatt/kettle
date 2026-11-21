//! The download queue: one active download at a time, the rest wait.
//!
//! The downloader (`sr_steam::content::Download`) runs on its own thread. A small pump thread per download
//! forwards its progress to the reducer as events tagged with the job number (the generation), at most four
//! times a second, and wakes the UI each time. When nothing downloads, nothing here runs, so idle CPU stays 0.
//!
//! Pause is cancel plus resume later: the downloader resumes by hashing what is already on disk.

use super::download_settings::{NotifyKind, SteamUpdatePolicy, UpdateBehavior};
use super::{Client, Event, Waker, shortcuts};
use sr_model::AppId;
use sr_steam::content::{self, Auth, Download, Options, Os, Progress, acf, bandwidth};
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

    pub(crate) fn os(self) -> Option<Os> {
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
    /// Language for new installs ("" = the setting, English by default).
    pub language: String,
    /// DLC app ids to install too.
    pub dlc: Vec<u32>,
    /// After a successful install: write a desktop icon / an application menu entry (`kettle --play <id>`).
    pub desktop_shortcut: bool,
    pub menu_shortcut: bool,
}

impl DownloadRequest {
    pub fn new(app: AppId, name: impl Into<String>, library: PathBuf) -> Self {
        DownloadRequest {
            app,
            name: name.into(),
            library,
            os: OsChoice::Auto,
            verify: false,
            language: String::new(),
            dlc: Vec::new(),
            desktop_shortcut: false,
            menu_shortcut: false,
        }
    }
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
    /// Fastest network speed seen during this download, bytes per second.
    pub peak_bytes_per_sec: f64,
    /// Bytes received from the network so far (compressed, as sent).
    pub network_bytes: u64,
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
    /// Network and disk speed of the last minute, one sample per second.
    pub speed: SpeedHistory,
    /// Downloads are held because a game started by Kettle is running (setting "Allow downloads during gameplay").
    pub gameplay_paused: bool,
}

/// How many one-second samples the speed graph keeps.
pub const SPEED_SAMPLES: usize = 60;

/// Speed of the last minute: `(network, disk)` bytes per second, oldest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpeedHistory {
    pub samples: VecDeque<(f32, f32)>,
}

impl SpeedHistory {
    pub fn push(&mut self, network: f32, disk: f32) {
        self.samples.push_back((network.max(0.0), disk.max(0.0)));
        while self.samples.len() > SPEED_SAMPLES {
            self.samples.pop_front();
        }
    }

    /// The largest value in the window (network or disk), for scaling the graph.
    pub fn max(&self) -> f32 {
        self.samples.iter().flat_map(|&(n, d)| [n, d]).fold(0.0, f32::max)
    }

    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

/// Move item `i` of `v` by `delta` places (negative = earlier), staying inside the list.
pub(crate) fn move_item<T>(v: &mut Vec<T>, i: usize, delta: i32) {
    if i >= v.len() {
        return;
    }
    let to = (i as i64 + i64::from(delta)).clamp(0, v.len() as i64 - 1) as usize;
    let item = v.remove(i);
    v.insert(to, item);
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
    /// For the speed graph: when, and the network and disk counters at the last sample.
    clock: SampleClock,
}

struct SampleClock {
    at: Instant,
    net: u64,
    disk: u64,
    net_start: u64,
}

impl Client {
    /// Add a download to the queue and start it when nothing else runs.
    pub(crate) fn dl_enqueue(&mut self, req: DownloadRequest) {
        self.dl_enqueue_at(req, false);
    }

    /// Like [`Self::dl_enqueue`]; `front` puts it before everything that waits (not before the running one).
    pub(crate) fn dl_enqueue_at(&mut self, req: DownloadRequest, front: bool) {
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
        let item = DownloadItem { request: req, kind };
        if front {
            self.state.downloads.queue.insert(0, item);
        } else {
            self.state.downloads.queue.push(item);
        }
        self.dl_start_next();
    }

    /// Queue a verify of an installed game, in the library it lives in.
    pub(crate) fn dl_verify(&mut self, app: AppId) {
        let Some((library, name)) = self.installed_in(app) else { return };
        if !self.may_write_library(app, &library) {
            return;
        }
        let mut req = DownloadRequest::new(app, name, library);
        req.verify = true;
        self.dl_enqueue(req);
    }

    /// Queue an update of an installed game, in its own library. A game in the official client's library
    /// folder is only touched when the setting allows it; with "ask" a dialog opens first.
    pub(crate) fn dl_update(&mut self, app: AppId) {
        let Some((library, name)) = self.installed_in(app) else { return };
        if !self.may_write_library(app, &library) {
            return;
        }
        self.dl_enqueue(DownloadRequest::new(app, name, library));
    }

    /// Update without asking (the user just answered the dialog, or the policy is "yes").
    pub(crate) fn dl_update_confirmed(&mut self, app: AppId, front: bool) {
        let Some((library, name)) = self.installed_in(app) else { return };
        self.dl_enqueue_at(DownloadRequest::new(app, name, library), front);
    }

    /// Queue the automatic update of `app` when its settings, the schedule and the library policy allow it.
    /// Returns whether it was queued.
    pub(crate) fn dl_auto_update(&mut self, app: AppId, hour: u8) -> bool {
        let d = &self.state.settings.downloads;
        let behavior = d.behavior_of(app);
        if behavior == UpdateBehavior::OnLaunch || !d.may_auto_update_at(hour) {
            return false;
        }
        let policy = d.steam_policy;
        let Some((library, _)) = self.installed_in(app) else { return false };
        if !self.is_our_library(&library) && policy != SteamUpdatePolicy::Yes {
            return false; // never silently write into the official client's folders
        }
        if self.state.downloads.status(app) != DownloadStatus::None {
            return false;
        }
        self.dl_update_confirmed(app, behavior == UpdateBehavior::HighPriority);
        true
    }

    /// Library and name of an installed game, with a notice when it can't be told.
    fn installed_in(&mut self, app: AppId) -> Option<(PathBuf, String)> {
        let Some(a) = self.installed.get(&app.0) else {
            self.state.notice = Some("This game is not installed.".into());
            return None;
        };
        let Some(library) = library_of(&a.install_dir) else {
            self.state.notice = Some("Can't tell which library this game is in.".into());
            return None;
        };
        Some((library, a.name.clone()))
    }

    /// May we write into `library` for `app`? Our own folders always; the official client's only when the
    /// setting says so. Without an answer yet it asks (the UI shows the dialog).
    fn may_write_library(&mut self, app: AppId, library: &Path) -> bool {
        if self.is_our_library(library) {
            return true;
        }
        match self.state.settings.downloads.steam_policy {
            SteamUpdatePolicy::Yes => true,
            SteamUpdatePolicy::Never => {
                self.state.notice = Some(
                    "Games installed by the Steam client are not changed (Settings > Downloads). Use the Steam client to update or verify them."
                        .into(),
                );
                false
            }
            SteamUpdatePolicy::Ask => {
                self.state.extra.steam_update_prompt = Some(app);
                false
            }
        }
    }

    /// Put a queued download first (it starts after the running one).
    pub(crate) fn dl_prioritize(&mut self, app: AppId) {
        let q = &mut self.state.downloads.queue;
        if let Some(i) = q.iter().position(|x| x.request.app == app) {
            move_item(q, i, -(i as i32));
        }
    }

    /// Move a queued download up (`delta` < 0) or down.
    pub(crate) fn dl_move(&mut self, app: AppId, delta: i32) {
        let q = &mut self.state.downloads.queue;
        if let Some(i) = q.iter().position(|x| x.request.app == app) {
            move_item(q, i, delta);
        }
    }

    /// Pause the running download and everything that waits.
    pub(crate) fn dl_pause_all(&mut self) {
        let d = &mut self.state.downloads;
        let waiting: Vec<DownloadItem> = std::mem::take(&mut d.queue);
        d.paused.extend(waiting.into_iter().map(|item| PausedDownload { item, done_bytes: 0, total_bytes: 0 }));
        if let Some(app) = d.active.as_ref().map(|a| a.item.request.app) {
            self.dl_stop_active(app, StopKind::Pause);
        }
    }

    /// Put every paused download back in the queue.
    pub(crate) fn dl_resume_all(&mut self) {
        let d = &mut self.state.downloads;
        let items: Vec<DownloadItem> = d.paused.drain(..).map(|p| p.item).collect();
        d.queue.extend(items);
        self.dl_start_next();
    }

    /// Apply the speed limit, and hold downloads while a game runs when the setting says so.
    pub(crate) fn apply_bandwidth(&mut self) {
        let d = &self.state.settings.downloads;
        bandwidth::set_limit(d.limit_bytes_per_sec());
        let hold = !d.allow_during_gameplay && !self.state.running.is_empty();
        bandwidth::set_paused(hold);
        self.state.downloads.gameplay_paused = hold && self.state.downloads.active.is_some();
    }

    pub(crate) fn dl_pause(&mut self, app: AppId) {
        // A waiting download is just moved to the paused list.
        let d = &mut self.state.downloads;
        if let Some(i) = d.queue.iter().position(|x| x.request.app == app) {
            let item = d.queue.remove(i);
            d.paused.push(PausedDownload { item, done_bytes: 0, total_bytes: 0 });
            return;
        }
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
        opts.language = if r.language.is_empty() {
            self.state.settings.downloads.language_or_default().to_string()
        } else {
            r.language.clone()
        };
        opts.dlc = r.dlc.clone();
        if item.kind != DownloadKind::Install
            && let Some(beta) = acf::load(&r.library, r.app.0).map(|s| s.beta_key).filter(|b| !b.is_empty())
        {
            opts.branch = beta; // keep a game on its beta branch
        }
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
        let net = bandwidth::net_bytes();
        let clock = SampleClock { at: Instant::now(), net, disk: 0, net_start: net };
        self.dl_running = Some(RunningDl { job, ctl, clock });
        self.state.downloads.speed.clear();
        self.state.downloads.active = Some(ActiveDownload {
            item,
            phase: DownloadPhase::Preparing,
            done_bytes: 0,
            total_bytes: 0,
            bytes_per_sec: 0.0,
            message: None,
            stopping: None,
            peak_bytes_per_sec: 0.0,
            network_bytes: 0,
        });
        self.apply_bandwidth();
    }

    pub(crate) fn dl_progress(&mut self, job: u64, p: Progress) {
        if self.dl_running.as_ref().map(|r| r.job) != Some(job) {
            return; // stale: a job that already ended
        }
        let done = p.done_bytes;
        if let Some(a) = &mut self.state.downloads.active {
            (a.phase, a.done_bytes, a.total_bytes, a.bytes_per_sec) =
                (p.phase, p.done_bytes, p.total_bytes, p.bytes_per_sec);
            if p.message.is_some() {
                a.message = p.message;
            }
        }
        // One graph sample per second: network from the shared counter, disk from the bytes handled.
        let (Some(run), Some(a)) = (&mut self.dl_running, &mut self.state.downloads.active) else { return };
        let now = Instant::now();
        let secs = now.duration_since(run.clock.at).as_secs_f64();
        if secs < 1.0 {
            return;
        }
        let net_now = bandwidth::net_bytes();
        let network = net_now.saturating_sub(run.clock.net) as f64 / secs;
        let disk = done.saturating_sub(run.clock.disk) as f64 / secs;
        run.clock = SampleClock { at: now, net: net_now, disk: done, net_start: run.clock.net_start };
        a.peak_bytes_per_sec = a.peak_bytes_per_sec.max(network);
        a.network_bytes = net_now.saturating_sub(run.clock.net_start);
        self.state.downloads.speed.push(network as f32, disk as f32);
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
                self.state.extra.updates.remove(&item.request.app.0);
                let r = &item.request;
                if item.kind == DownloadKind::Install && (r.desktop_shortcut || r.menu_shortcut) {
                    self.make_shortcuts(r.app, name.clone(), r.desktop_shortcut, r.menu_shortcut);
                }
                if item.kind != DownloadKind::Verify {
                    let what = if item.kind == DownloadKind::Update { "was updated" } else { "is ready to play" };
                    self.notify(NotifyKind::DownloadDone, "Download finished", &format!("{name} {what}."));
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
        self.apply_bandwidth();
        // Whatever happened, the files on disk may have changed.
        self.spawn_local_scan();
    }

    /// Write the `.desktop` files on a worker thread and say what happened.
    pub(crate) fn make_shortcuts(&self, app: AppId, name: String, desktop: bool, menu: bool) {
        self.spawn_store(
            move || {
                let icon = sr_steam::local::steam_roots().iter().find_map(|r| sr_steam::local::cached_icon(r, app.0));
                shortcuts::create(&name, app.0, desktop, menu, icon.as_deref())
            },
            |r| {
                Event::Notice(match r {
                    Ok(files) if files.is_empty() => "No shortcut was asked for.".to_string(),
                    Ok(files) => format!("Shortcut created: {}", files[0].display()),
                    Err(e) => format!("Couldn't create the shortcut: {e}"),
                })
            },
        );
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

/// `<library>/steamapps/common/<dir>` as text (the form `InstallState` keeps) -> `<library>`.
pub fn library_of_path(install_dir: &str) -> Option<PathBuf> {
    library_of(Path::new(install_dir))
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
        (Err(e), None) => DlResult::Failed(friendly_error(&e)),
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
    let needed = content::estimate::with_margin(p.total_bytes);
    (needed > free).then(|| format!("Not enough disk space: {} needed, {} free.", human(needed), human(free)))
}

/// The error text for the user: a full disk gets a plain sentence, anything else the error chain.
pub(crate) fn friendly_error(e: &anyhow::Error) -> String {
    let full = e
        .chain()
        .any(|c| c.downcast_ref::<std::io::Error>().is_some_and(|io| io.kind() == std::io::ErrorKind::StorageFull));
    if full { "The disk is full. Free some space, then resume the download.".to_string() } else { format!("{e:#}") }
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
        let item =
            DownloadItem { request: DownloadRequest::new(AppId(1), "G", "/l".into()), kind: DownloadKind::Install };
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
            peak_bytes_per_sec: 0.0,
            network_bytes: 0,
        });
        assert_eq!(d.status(AppId(1)), DownloadStatus::Active(0.25));
        assert_eq!(d.active.as_ref().and_then(ActiveDownload::eta_secs), Some(15));
    }

    #[test]
    fn queue_reordering_stays_in_bounds() {
        let mut v = vec![1, 2, 3, 4];
        move_item(&mut v, 2, -1);
        assert_eq!(v, [1, 3, 2, 4]);
        move_item(&mut v, 0, -5);
        assert_eq!(v, [1, 3, 2, 4]);
        move_item(&mut v, 1, 10);
        assert_eq!(v, [1, 2, 4, 3]);
        move_item(&mut v, 3, -3);
        assert_eq!(v, [3, 1, 2, 4]);
        move_item(&mut v, 9, 1); // no such item: nothing happens
        assert_eq!(v, [3, 1, 2, 4]);
    }

    #[test]
    fn speed_history_keeps_a_minute() {
        let mut h = SpeedHistory::default();
        for i in 0..100 {
            h.push(i as f32, 1.0);
        }
        assert_eq!(h.samples.len(), SPEED_SAMPLES);
        assert_eq!(h.samples.front(), Some(&(40.0, 1.0)));
        assert_eq!(h.max(), 99.0);
        h.push(-5.0, 0.0);
        assert_eq!(h.samples.back(), Some(&(0.0, 0.0)));
    }

    #[test]
    fn a_full_disk_gets_a_plain_message() {
        let io = std::io::Error::from(std::io::ErrorKind::StorageFull);
        let e = anyhow::Error::new(io).context("write game.bin");
        assert!(friendly_error(&e).starts_with("The disk is full"));
        let other = anyhow::anyhow!("boom");
        assert_eq!(friendly_error(&other), "boom");
    }

    #[cfg(unix)]
    #[test]
    fn free_space_of_tmp_is_known() {
        assert!(free_space(Path::new("/tmp/does/not/exist")).is_some());
    }
}
