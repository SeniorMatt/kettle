//! Application state and the reducer.
//!
//! The UI owns a [`Client`] on its thread. Each frame it calls [`Client::poll`]
//! (apply finished background work), reads [`Client::state`], and sends
//! [`Command`]s. Background tasks never touch `State`: they send an [`Event`]
//! and wake the UI.
//!
//! Architecture invariant: tokens live only in `Client::session` (private) and
//! in the keyring, never in `State`.

mod art;
mod demo;
mod launch;

pub use art::ArtImage;

use sr_model::{Account, AppId, Game, InstallState, SteamId};
use sr_steam::{auth, local, secrets, store, webapi};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How the UI is woken from a background thread (`egui::Context::request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Events buffered between background tasks and the UI.
const EVENT_QUEUE: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Login(LoginState),
    Library,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LoginState {
    /// The `https://s.team/q/...` URL to show as a QR code.
    pub qr_url: Option<String>,
    /// The phone scanned the code and is waiting for "Approve".
    pub scanned: bool,
    pub error: Option<String>,
}

/// Everything the UI may read.
#[derive(Debug, Clone)]
pub struct State {
    pub screen: Screen,
    pub account: Option<Account>,
    /// Owned games, sorted by `sort_key`.
    pub games: Vec<Game>,
    /// Bumped whenever `games` changes, so views can cache derived lists.
    pub library_rev: u64,
    pub selected: Option<AppId>,
    /// A refresh of the owned list is running.
    pub refreshing: bool,
    /// Last non-fatal problem, shown in the status bar.
    pub notice: Option<String>,
    pub timings: Timings,
}

/// Startup milestones, for the performance log.
#[derive(Debug, Clone, Default)]
pub struct Timings {
    pub cached_library: Option<Duration>,
    pub local_scan: Option<Duration>,
    pub owned_fetch: Option<Duration>,
}

impl State {
    pub fn game(&self, app: AppId) -> Option<&Game> {
        self.games.iter().find(|g| g.app_id == app)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    StartQrLogin,
    Logout,
    RefreshLibrary,
    Select(Option<AppId>),
    Play(AppId),
    Install(AppId),
}

/// Results of background work. `generation` ties a result to the session that
/// asked for it; results from an older generation are dropped.
#[derive(Debug)]
enum Event {
    Qr { generation: u64, url: String },
    QrScanned { generation: u64 },
    LoggedIn { generation: u64, tokens: auth::Tokens },
    Owned { generation: u64, games: Vec<webapi::OwnedGame>, took: Duration },
    Installed { apps: HashMap<u32, local::InstalledApp>, took: Duration },
    LoginFailed { generation: u64, error: String },
    Notice(String),
}

struct Session {
    account: Account,
    refresh_token: String,
}

impl Drop for Session {
    /// Wipe the token from memory when the session ends.
    fn drop(&mut self) {
        // SAFETY: zero bytes are valid UTF-8.
        unsafe { self.refresh_token.as_bytes_mut() }.fill(0);
    }
}

pub struct Client {
    state: State,
    session: Option<Session>,
    installed: HashMap<u32, local::InstalledApp>,
    rt: tokio::runtime::Runtime,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    wake: Waker,
    generation: Arc<AtomicU64>,
    art: art::ArtLoader,
}

impl Client {
    pub fn new(wake: Waker) -> anyhow::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let (tx, rx) = mpsc::channel(EVENT_QUEUE);
        let roots = local::steam_roots();
        let mut c = Self {
            state: State {
                screen: Screen::Login(LoginState::default()),
                account: None,
                games: Vec::new(),
                library_rev: 0,
                selected: None,
                refreshing: false,
                notice: None,
                timings: Timings::default(),
            },
            session: None,
            installed: HashMap::new(),
            art: art::ArtLoader::new(roots, wake.clone()),
            rt,
            tx,
            rx,
            wake,
            generation: Arc::new(AtomicU64::new(0)),
        };
        c.spawn_local_scan();
        match std::env::var("STEAM_RUST_DEMO").ok().and_then(|v| v.parse::<usize>().ok()) {
            Some(n) => c.start_demo(n),
            None => c.resume_or_login(),
        }
        Ok(c)
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Apply finished background work. Call once per frame; cheap when idle.
    pub fn poll(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.apply(ev);
        }
    }

    pub fn send(&mut self, cmd: Command) {
        match cmd {
            Command::StartQrLogin => self.start_qr_login(),
            Command::Logout => self.logout(),
            Command::RefreshLibrary => self.spawn_refresh(),
            Command::Select(app) => self.state.selected = app,
            Command::Play(app) => self.notice_on_err(launch::play(app)),
            Command::Install(app) => self.notice_on_err(launch::install(app)),
        }
    }

    /// Ask for cover art. Cheap to call every frame for visible items; each
    /// image is loaded at most once until it's evicted by the UI.
    pub fn request_art(&self, app: AppId, kind: sr_model::ArtKind) {
        self.art.request(app, kind);
    }

    /// Tell the loader an image was dropped by the UI so it can be re-requested.
    pub fn forget_art(&self, app: AppId, kind: sr_model::ArtKind) {
        self.art.forget(app, kind);
    }

    /// Decoded images that finished since the last call.
    pub fn take_art(&self) -> Vec<ArtImage> {
        self.art.take()
    }

    // ---- reducer -------------------------------------------------------

    fn apply(&mut self, ev: Event) {
        let current = self.generation.load(Ordering::Relaxed);
        match ev {
            Event::Qr { generation, url } if generation == current => {
                if let Screen::Login(l) = &mut self.state.screen {
                    l.qr_url = Some(url);
                    l.scanned = false;
                    l.error = None;
                }
            }
            Event::QrScanned { generation } if generation == current => {
                if let Screen::Login(l) = &mut self.state.screen {
                    l.scanned = true;
                }
            }
            Event::LoggedIn { generation, mut tokens } if generation == current => {
                let account = Account { account_name: tokens.account_name.clone(), steam_id: SteamId(tokens.steamid) };
                if let Err(e) = secrets::save_refresh_token(&account.account_name, &tokens.refresh_token) {
                    self.state.notice = Some(format!("Couldn't save the login to the keyring: {e:#}"));
                }
                remember_account(&account);
                self.begin_session(account, std::mem::take(&mut tokens.refresh_token));
            }
            Event::LoginFailed { generation, error } if generation == current => {
                if let Screen::Login(l) = &mut self.state.screen {
                    l.error = Some(error);
                    l.qr_url = None;
                }
            }
            Event::Owned { generation, games, took } if generation == current => {
                self.state.refreshing = false;
                self.state.timings.owned_fetch = Some(took);
                let mut list: Vec<Game> = games
                    .into_iter()
                    .map(|g| {
                        let mut game = Game::new(AppId(g.appid), g.name);
                        game.playtime_minutes = g.playtime_forever;
                        game.last_played = g.rtime_last_played;
                        game.icon_hash = g.img_icon_url;
                        game
                    })
                    .collect();
                list.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
                if let Some(acct) = &self.state.account {
                    let _ = store::save_json(&library_cache_path(acct.steam_id), &list);
                }
                self.state.games = list;
                self.merge_installed();
            }
            Event::Installed { apps, took } => {
                self.state.timings.local_scan = Some(took);
                self.installed = apps;
                self.merge_installed();
            }
            Event::Notice(msg) => self.state.notice = Some(msg),
            // Stale result from an older generation.
            _ => {}
        }
    }

    /// Recompute install state and notify views that `games` changed.
    fn merge_installed(&mut self) {
        self.state.library_rev += 1;
        self.art.set_icon_hashes(self.state.games.iter().map(|g| (g.app_id, g.icon_hash.clone())).collect());
        for g in &mut self.state.games {
            g.install = match self.installed.get(&g.app_id.0) {
                Some(a) => {
                    let path = a.install_dir.display().to_string();
                    if a.state_flags & 2 != 0 {
                        InstallState::UpdateRequired { size_bytes: a.size_on_disk, path }
                    } else {
                        InstallState::Installed { size_bytes: a.size_on_disk, path }
                    }
                }
                None => InstallState::NotInstalled,
            };
        }
    }

    fn notice_on_err(&mut self, r: anyhow::Result<()>) {
        if let Err(e) = r {
            self.state.notice = Some(format!("{e:#}"));
        }
    }

    // ---- session -------------------------------------------------------

    fn bump_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Continue the last session from the keyring, or show the QR login.
    fn resume_or_login(&mut self) {
        if let Some(account) = remembered_accounts().into_iter().next() {
            match secrets::load_refresh_token(&account.account_name) {
                Ok(Some(token)) if !token_expired(&token) => return self.begin_session(account, token),
                Ok(_) => {}
                Err(e) => self.state.notice = Some(format!("Keyring: {e:#}")),
            }
        }
        self.start_qr_login();
    }

    fn begin_session(&mut self, account: Account, refresh_token: String) {
        self.bump_generation();
        let t = Instant::now();
        if let Some(cached) = store::load_json::<Vec<Game>>(&library_cache_path(account.steam_id)) {
            self.state.games = cached;
            self.state.timings.cached_library = Some(t.elapsed());
            self.merge_installed();
        }
        self.state.account = Some(account.clone());
        self.state.screen = Screen::Library;
        self.session = Some(Session { account, refresh_token });
        self.spawn_refresh();
    }

    fn start_demo(&mut self, n: usize) {
        let t = Instant::now();
        self.state.games = demo::games(n);
        self.state.timings.cached_library = Some(t.elapsed());
        self.state.account = Some(Account { account_name: "demo".into(), steam_id: SteamId(0) });
        self.state.screen = Screen::Library;
        self.merge_installed();
    }

    fn logout(&mut self) {
        if let Some(s) = self.session.take() {
            self.notice_on_err(secrets::delete_refresh_token(&s.account.account_name));
            forget_account(&s.account);
            let _ = std::fs::remove_file(library_cache_path(s.account.steam_id));
            // Invalidate the token on Steam's side too (best effort, off-thread).
            let token = s.refresh_token.clone();
            self.rt.spawn_blocking(move || {
                let mut token = token;
                let _ = auth::revoke_refresh_token(&token);
                // SAFETY: zero bytes are valid UTF-8.
                unsafe { token.as_bytes_mut() }.fill(0);
            });
        }
        self.state.account = None;
        self.state.games.clear();
        self.state.library_rev += 1;
        self.state.selected = None;
        self.start_qr_login();
    }

    fn start_qr_login(&mut self) {
        let generation = self.bump_generation();
        self.state.screen = Screen::Login(LoginState::default());
        let (tx, wake, current) = (self.tx.clone(), self.wake.clone(), self.generation.clone());
        self.rt.spawn_blocking(move || {
            let send = |ev| {
                let _ = tx.blocking_send(ev);
                wake();
            };
            let fail = |e: anyhow::Error| Event::LoginFailed { generation, error: format!("{e:#}") };
            let mut session = match auth::begin_qr("steam-rust (Linux)") {
                Ok(s) => s,
                Err(e) => return send(fail(e)),
            };
            send(Event::Qr { generation, url: session.challenge_url.clone() });
            let deadline = Instant::now() + Duration::from_secs(15 * 60);
            // Stop as soon as a newer generation exists (cancel, logout, retry).
            while current.load(Ordering::Relaxed) == generation {
                if Instant::now() > deadline {
                    return send(Event::LoginFailed { generation, error: "The QR code expired.".into() });
                }
                std::thread::sleep(session.interval);
                match session.poll() {
                    Ok(auth::Poll::Waiting) => {}
                    Ok(auth::Poll::Scanned) => send(Event::QrScanned { generation }),
                    Ok(auth::Poll::NewChallenge(url)) => send(Event::Qr { generation, url }),
                    Ok(auth::Poll::Done(tokens)) => return send(Event::LoggedIn { generation, tokens }),
                    Err(e) => return send(fail(e)),
                }
            }
        });
    }

    fn spawn_refresh(&mut self) {
        let Some(s) = &self.session else { return };
        self.state.refreshing = true;
        let generation = self.generation.load(Ordering::Relaxed);
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let (token, steamid) = (s.refresh_token.clone(), s.account.steam_id.0);
        self.rt.spawn_blocking(move || {
            let t = Instant::now();
            let ev = match auth::renew_access_token(&token).and_then(|access| webapi::owned_games(&access, steamid)) {
                Ok(games) => Event::Owned { generation, games, took: t.elapsed() },
                Err(e) => Event::Notice(format!("Couldn't refresh the library: {e:#}")),
            };
            let _ = tx.blocking_send(ev);
            wake();
        });
    }

    fn spawn_local_scan(&self) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let t = Instant::now();
            let apps = local::installed_apps();
            let _ = tx.blocking_send(Event::Installed { apps, took: t.elapsed() });
            wake();
        });
    }
}

/// Refresh tokens are JWTs; treat one that expires within a day as expired.
fn token_expired(token: &str) -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    auth::jwt_expiry(token).is_none_or(|exp| exp < now + 86_400)
}

fn accounts_path() -> std::path::PathBuf {
    store::data_dir().join("accounts.json")
}

fn library_cache_path(id: SteamId) -> std::path::PathBuf {
    store::cache_dir().join(format!("library-{id}.json"))
}

/// Known accounts, most recently used first. Names and SteamIDs only.
fn remembered_accounts() -> Vec<Account> {
    store::load_json(&accounts_path()).unwrap_or_default()
}

fn remember_account(a: &Account) {
    let mut list = remembered_accounts();
    list.retain(|x| x.steam_id != a.steam_id);
    list.insert(0, a.clone());
    let _ = store::save_json(&accounts_path(), &list);
}

fn forget_account(a: &Account) {
    let mut list = remembered_accounts();
    list.retain(|x| x.steam_id != a.steam_id);
    let _ = store::save_json(&accounts_path(), &list);
}
