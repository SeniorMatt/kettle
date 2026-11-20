//! Game downloader: installs, updates and verifies a Steam app the way Steam
//! does, into a Steam-format library folder, from Steam's own content servers.
//!
//! Pipeline: PICS app info -> depot selection -> depot keys, manifest codes and
//! CDN servers over the CM connection -> manifests -> chunks over HTTPS ->
//! files on disk -> `appmanifest_<id>.acf`. No GUI types here; drive it with
//! [`download`] (async, any connection) or [`Download::start`] (own thread).

pub mod acf;
pub mod cdn;
pub mod cli;
mod compress;
mod crypto;
pub mod depots;
mod install;
pub mod manifest;
pub mod product;

use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use steam_vent::{Connection, ConnectionTrait, ServerList};

pub use depots::Os;
pub use install::Stats;
pub use product::{AppInfo, LaunchEntry};

/// A CM connection that can run content-system requests. Anonymous and
/// signed-in connections are both a `steam_vent::Connection`; the trait keeps
/// the downloader independent of how the connection was made.
pub trait CmLink: ConnectionTrait + Sync {
    /// Steam's cell (region) id of this session, used to pick nearby CDN servers.
    fn cell_id(&self) -> u32;
}

impl CmLink for Connection {
    fn cell_id(&self) -> u32 {
        Connection::cell_id(self)
    }
}

/// Where our own downloads go unless the caller says otherwise.
pub fn default_library() -> PathBuf {
    crate::store::data_dir().join("library")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    Preparing,
    Downloading,
    Verifying,
    Finished,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Progress {
    pub app: u32,
    pub phase: Phase,
    /// Bytes handled in this phase (hashed while verifying, written while downloading).
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_sec: f64,
    /// Latest warning (skipped depot, ...) or the error text for `Failed`.
    pub message: Option<String>,
}

/// Share a clone with whoever may cancel; the download stops within about a chunk.
#[derive(Debug, Clone, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// The error a cancelled download returns; test with [`is_cancelled`].
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("download cancelled")
    }
}

impl std::error::Error for Cancelled {}

pub fn is_cancelled(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Cancelled>().is_some()
}

#[derive(Debug, Clone)]
pub struct Options {
    pub app_id: u32,
    /// Library root; files go to `<library>/steamapps/common/<installdir>`.
    pub library: PathBuf,
    /// Target OS; `None` picks Linux when the app has a Linux build, else Windows.
    pub os: Option<Os>,
    pub language: String,
    pub branch: String,
    /// Hash every file and re-fetch only bad or missing chunks.
    pub verify: bool,
    /// Chunk downloads in flight.
    pub workers: usize,
    /// DLC app ids whose depots to install too.
    pub dlc: Vec<u32>,
    /// SteamID64 recorded as `LastOwner` (0 for anonymous).
    pub owner: u64,
}

impl Options {
    pub fn new(app_id: u32, library: impl Into<PathBuf>) -> Self {
        Options {
            app_id,
            library: library.into(),
            os: None,
            language: "english".into(),
            branch: "public".into(),
            verify: false,
            workers: 8,
            dlc: Vec::new(),
            owner: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub app: AppInfo,
    pub os: Os,
    pub install_dir: PathBuf,
    /// depot -> manifest gid now installed.
    pub depots: BTreeMap<u32, u64>,
    pub stats: Stats,
    pub seconds: f64,
    /// Nothing to do: the installed build already matched.
    pub up_to_date: bool,
    pub warnings: Vec<String>,
}

/// Turns raw byte counts into a smoothed rate and rate-limited callbacks.
struct Reporter {
    app: u32,
    on: Arc<dyn Fn(Progress) + Send + Sync>,
    samples: std::collections::VecDeque<(Instant, u64)>,
    last_phase: Option<Phase>,
    message: Option<String>,
}

impl Reporter {
    fn new(app: u32, on: Arc<dyn Fn(Progress) + Send + Sync>) -> Self {
        Reporter { app, on, samples: Default::default(), last_phase: None, message: None }
    }

    fn emit(&mut self, phase: Phase, done: u64, total: u64) {
        let now = Instant::now();
        if self.last_phase != Some(phase) {
            self.samples.clear();
            self.last_phase = Some(phase);
        }
        self.samples.push_back((now, done));
        while self.samples.front().is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(3)) {
            self.samples.pop_front();
        }
        let rate = match (self.samples.front(), self.samples.back()) {
            (Some((t0, b0)), Some((t1, b1))) if t1 > t0 => (b1 - b0) as f64 / t1.duration_since(*t0).as_secs_f64(),
            _ => 0.0,
        };
        (self.on)(Progress {
            app: self.app,
            phase,
            done_bytes: done,
            total_bytes: total,
            bytes_per_sec: rate,
            message: self.message.clone(),
        });
    }
}

/// Install, update or verify `opts.app_id` over an open Steam connection.
/// Progress goes to `on_progress` (from the task's thread, a few times a second).
pub async fn download<C: CmLink>(
    conn: &C,
    opts: &Options,
    cancel: &CancelToken,
    on_progress: impl Fn(Progress) + Send + Sync + 'static,
) -> Result<Summary> {
    let mut rep = Reporter::new(opts.app_id, Arc::new(on_progress));
    rep.emit(Phase::Preparing, 0, 0);
    match run(conn, opts, cancel, &mut rep).await {
        Ok(s) => {
            let total = s.stats.bytes_downloaded;
            rep.emit(Phase::Finished, total, total);
            Ok(s)
        }
        Err(e) => {
            rep.message = Some(format!("{e:#}"));
            rep.emit(Phase::Failed, 0, 0);
            Err(e)
        }
    }
}

/// A depot chosen for install with the app that owns its content.
struct Chosen {
    depot: u32,
    app: u32,
    gid: u64,
}

async fn run<C: CmLink>(conn: &C, opts: &Options, cancel: &CancelToken, rep: &mut Reporter) -> Result<Summary> {
    let started = Instant::now();
    let app = product::fetch(conn, opts.app_id).await?;
    if app.branches.get(&opts.branch).is_some_and(|b| b.password_required) {
        bail!("branch {:?} needs a password", opts.branch);
    }
    let os = opts.os.unwrap_or_else(|| Os::default_for(&app));
    let target = depots::Target { os, language: &opts.language, branch: &opts.branch, dlc: &opts.dlc };
    let mut warnings = Vec::new();
    let chosen = resolve(conn, &app, depots::select(&app, &target), &opts.branch, &mut warnings).await?;
    if chosen.is_empty() {
        bail!("app {} has no depot for {} / {} on branch {:?}", app.appid, os.as_str(), opts.language, opts.branch);
    }
    let install_dir_name =
        if app.install_dir.is_empty() { format!("app_{}", app.appid) } else { app.install_dir.clone() };
    let dir = opts.library.join("steamapps/common").join(&install_dir_name);
    let installed = acf::load(&opts.library, opts.app_id);
    let gids: BTreeMap<u32, u64> = chosen.iter().map(|c| (c.depot, c.gid)).collect();
    let summary = |stats, up_to_date, warnings, depots| Summary {
        app: app.clone(),
        os,
        install_dir: dir.clone(),
        depots,
        stats,
        seconds: started.elapsed().as_secs_f64(),
        up_to_date,
        warnings,
    };
    if let Some(inst) = installed.as_ref().filter(|_| !opts.verify) {
        let same = inst.state_flags & acf::STATE_INSTALLED != 0
            && dir.is_dir()
            && gids.iter().all(|(d, g)| inst.depots.get(d).is_some_and(|(have, _)| have == g));
        if same {
            return Ok(summary(Stats::default(), true, warnings, gids));
        }
    }

    let cdn = Arc::new(cdn::servers(conn).await?);
    let mut plans = Vec::new();
    for c in &chosen {
        if cancel.is_cancelled() {
            return Err(Cancelled.into());
        }
        let Some(key) = cdn::depot_key(conn, c.app, c.depot).await? else {
            warnings.push(format!("depot {}: no access, skipped", c.depot));
            continue;
        };
        let code = cdn::manifest_request_code(conn, c.app, c.depot, c.gid, &opts.branch).await?;
        let new = fetch_manifest(&cdn, c.depot, c.gid, code, key).await?;
        let old = old_manifest(&opts.library, installed.as_ref(), &new, &key);
        plans.push(install::DepotPlan { key, new, old });
    }
    if plans.is_empty() {
        bail!("no depot of app {} could be downloaded: {}", app.appid, warnings.join("; "));
    }
    rep.message = warnings.last().cloned();

    let state = acf::AppState {
        appid: app.appid,
        name: app.name.clone(),
        install_dir: install_dir_name,
        state_flags: acf::STATE_UPDATING,
        last_updated: installed.as_ref().map_or(0, |i| i.last_updated),
        size_on_disk: installed.as_ref().map_or(0, |i| i.size_on_disk),
        build_id: installed.as_ref().map_or(0, |i| i.build_id),
        last_owner: opts.owner,
        depots: installed.as_ref().map(|i| i.depots.clone()).unwrap_or_default(),
    };
    std::fs::create_dir_all(&dir)?;
    acf::save(&opts.library, &state)?;
    let marker = write_marker(&opts.library, &app, &plans, &opts.branch)?;

    let (plans, stats) = apply_blocking(plans, cdn, dir.clone(), opts, cancel, rep).await?;

    let mut state = state;
    for p in &plans {
        save_cache(&opts.library, &p.new, installed.as_ref())?;
        state.depots.insert(p.new.depot, (p.new.gid, p.new.total_size));
    }
    state.state_flags = acf::STATE_INSTALLED;
    state.last_updated = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    state.size_on_disk = install::dir_size(&dir);
    state.build_id = app.build_id(&opts.branch);
    acf::save(&opts.library, &state)?;
    let _ = std::fs::remove_dir_all(marker);
    let depots = plans.iter().map(|p| (p.new.depot, p.new.gid)).collect();
    Ok(summary(stats, false, warnings, depots))
}

/// Run the blocking install on a worker thread, forwarding its progress.
async fn apply_blocking(
    plans: Vec<install::DepotPlan>,
    cdn: Arc<cdn::Cdn>,
    dir: PathBuf,
    opts: &Options,
    cancel: &CancelToken,
    rep: &mut Reporter,
) -> Result<(Vec<install::DepotPlan>, Stats)> {
    let (workers, verify, cancel) = (opts.workers, opts.verify, cancel.clone());
    let shared = Arc::new(Mutex::new(std::mem::replace(rep, Reporter::new(opts.app_id, Arc::new(|_| {})))));
    let inner = shared.clone();
    let joined = tokio::task::spawn_blocking(move || {
        let settings = install::Settings { dir: &dir, cdn: &cdn, cancel: &cancel, workers, verify };
        let mut report = move |phase, done, total| inner.lock().unwrap().emit(phase, done, total);
        let stats = install::apply(&settings, &plans, &mut report)?;
        anyhow::Ok((plans, stats))
    })
    .await;
    // Hand the reporter (with its rate history) back to the caller.
    if let Some(m) = Arc::into_inner(shared) {
        *rep = m.into_inner().unwrap();
    }
    joined.context("install task")?
}

/// Pair every wanted depot with the app and manifest that provide it.
async fn resolve<C: CmLink>(
    conn: &C,
    app: &AppInfo,
    picked: Vec<&product::DepotInfo>,
    branch: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<Chosen>> {
    let mut others: HashMap<u32, AppInfo> = HashMap::new();
    let mut out = Vec::new();
    for d in picked {
        let (owner, gid) = match (d.manifests.get(branch), d.from_app) {
            (Some(g), None) => (app.appid, *g),
            (own, Some(from)) => {
                if let std::collections::hash_map::Entry::Vacant(slot) = others.entry(from) {
                    slot.insert(product::fetch(conn, from).await?);
                }
                let theirs = others[&from].depot(d.id).and_then(|x| x.manifests.get(branch)).copied();
                match theirs.or(own.copied()) {
                    Some(g) => (from, g),
                    None => {
                        warnings.push(format!("depot {}: no manifest on {branch:?}, skipped", d.id));
                        continue;
                    }
                }
            }
            (None, None) => continue,
        };
        out.push(Chosen { depot: d.id, app: owner, gid });
    }
    Ok(out)
}

async fn fetch_manifest(
    cdn: &Arc<cdn::Cdn>,
    depot: u32,
    gid: u64,
    code: u64,
    key: crypto::DepotKey,
) -> Result<manifest::Manifest> {
    let path = if code == 0 {
        format!("/depot/{depot}/manifest/{gid}/5")
    } else {
        format!("/depot/{depot}/manifest/{gid}/5/{code}")
    };
    let cdn = cdn.clone();
    tokio::task::spawn_blocking(move || {
        let zip = cdn.get(&path, 256 << 20).with_context(|| format!("manifest {gid} of depot {depot}"))?;
        let m = manifest::from_zip(&zip, &key)?;
        anyhow::ensure!(m.gid == gid, "Steam sent manifest {} instead of {gid}", m.gid);
        Ok(m)
    })
    .await?
}

fn cache_path(library: &Path, depot: u32, gid: u64) -> PathBuf {
    library.join("depotcache").join(format!("{depot}_{gid}.manifest"))
}

/// The manifest of what is installed now, when it differs from the new one and was cached.
fn old_manifest(
    library: &Path,
    installed: Option<&acf::AppState>,
    new: &manifest::Manifest,
    key: &crypto::DepotKey,
) -> Option<manifest::Manifest> {
    let (gid, _) = *installed?.depots.get(&new.depot)?;
    if gid == new.gid {
        return Some(new.clone());
    }
    manifest::parse(std::fs::read(cache_path(library, new.depot, gid)).ok()?, key).ok()
}

/// Keep the new manifest in Steam's depotcache layout and drop the one it replaces.
fn save_cache(library: &Path, m: &manifest::Manifest, previous: Option<&acf::AppState>) -> Result<()> {
    let p = cache_path(library, m.depot, m.gid);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, &m.raw)?;
    if let Some((gid, _)) = previous.and_then(|i| i.depots.get(&m.depot)).filter(|(g, _)| *g != m.gid) {
        let _ = std::fs::remove_file(cache_path(library, m.depot, *gid));
    }
    Ok(())
}

/// `steamapps/downloading/<appid>/state.json` exists exactly while an install is unfinished.
fn write_marker(library: &Path, app: &AppInfo, plans: &[install::DepotPlan], branch: &str) -> Result<PathBuf> {
    let dir = library.join("steamapps/downloading").join(app.appid.to_string());
    std::fs::create_dir_all(&dir)?;
    let depots: BTreeMap<u32, u64> = plans.iter().map(|p| (p.new.depot, p.new.gid)).collect();
    let json =
        serde_json::json!({ "appid": app.appid, "branch": branch, "buildid": app.build_id(branch), "depots": depots });
    std::fs::write(dir.join("state.json"), serde_json::to_vec_pretty(&json)?)?;
    Ok(dir)
}

/// An anonymous CM session; Steam's directory lookup fails now and then, so try a few times.
pub async fn anonymous_connection() -> Result<Connection> {
    let mut last = None;
    for attempt in 0..4u64 {
        tokio::time::sleep(Duration::from_secs(attempt)).await;
        let servers = match ServerList::discover().await {
            Ok(s) => s,
            Err(e) => {
                last = Some(anyhow::Error::new(e).context("CM server discovery"));
                continue;
            }
        };
        match Connection::anonymous(&servers).await {
            Ok(c) => return Ok(c),
            Err(e) => last = Some(anyhow::Error::new(e).context("anonymous logon")),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no attempt made")))
}

/// How to sign in for [`Download::start`].
#[derive(Clone)]
pub enum Auth {
    Anonymous,
    Account { name: String, refresh_token: String },
}

/// A download running on its own thread (with its own async runtime).
pub struct Download {
    pub progress: Receiver<Progress>,
    cancel: CancelToken,
    thread: std::thread::JoinHandle<Result<Summary>>,
}

impl Download {
    pub fn start(mut opts: Options, auth: Auth) -> Download {
        let (tx, progress): (Sender<Progress>, _) = channel();
        let cancel = CancelToken::new();
        let token = cancel.clone();
        if let Auth::Account { refresh_token, .. } = &auth {
            opts.owner = crate::auth::jwt_steamid(refresh_token).unwrap_or(0);
        }
        let thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
            let tx = Mutex::new(tx);
            let send = move |p: Progress| {
                let _ = tx.lock().unwrap().send(p);
            };
            rt.block_on(async {
                match auth {
                    Auth::Anonymous => {
                        let conn = anonymous_connection().await?;
                        download(&conn, &opts, &token, send).await
                    }
                    Auth::Account { name, refresh_token } => {
                        let cm = crate::cm::connect(&name, &refresh_token).await?;
                        download(&cm.conn, &opts, &token, send).await
                    }
                }
            })
        });
        Download { progress, cancel, thread }
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_finished(&self) -> bool {
        self.thread.is_finished()
    }

    /// Wait for the end and get the result.
    pub fn join(self) -> Result<Summary> {
        self.thread.join().map_err(|_| anyhow::anyhow!("download thread panicked"))?
    }
}
