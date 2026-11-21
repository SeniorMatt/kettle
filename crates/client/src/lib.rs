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
mod cm_hub;
mod collections;
mod demo;
mod download_settings;
mod downloads;
mod extras;
mod friends;
mod gamescope_settings;
mod hud_settings;
mod interface_settings;
mod launch;
mod launch_options;
mod manage;
mod notes;
mod notify;
mod open;
mod overlay_proto;
mod play;
mod profile;
mod settings;
mod shortcuts;
#[cfg(target_os = "linux")]
mod steamworks_host;
mod storage;
mod sysinfo;
mod tools;
mod web_image;

pub use art::ArtImage;
pub use collections::{CollCmd, Collections, FAVORITE, HIDDEN};
pub use download_settings::{
    DownloadSettings, NotifyKind, NotifySettings, SteamUpdatePolicy, UpdateBehavior, hour_in_window, local_hour,
    rate_text,
};
pub use downloads::{
    ActiveDownload, DownloadItem, DownloadKind, DownloadPhase, DownloadRequest, DownloadStatus, DownloadsState,
    FinishedDownload, OsChoice, Outcome, PausedDownload, StopKind,
};
pub use downloads::{SPEED_SAMPLES, SpeedHistory, free_space, library_of_path};
pub use extras::{DlcInfo, ExtraState, InstallInfo, ManageCmd, UpdateCheck, with_margin};
pub use friends::{
    Chat, Delivery, Friend, FriendAction, FriendsState, FriendsStatus, GamePlayed, GroupChat, GroupInfo, GroupMessage,
    GroupsState, HistoryState, LOADING as NAME_LOADING, MAX_GROUP_MESSAGES, MAX_TEXT, Me, Message, PersonaState,
    RoomInfo, Toast, local_utc_offset, sort_key as friend_sort_key,
};
pub use gamescope_settings::{
    Detection as GamescopeDetection, GamescopeGames, GamescopeSettings, INSTALL_HINTS as GAMESCOPE_INSTALL_HINTS,
    INSTALL_URL as GAMESCOPE_INSTALL_URL, MAX_SHARPNESS as GAMESCOPE_MAX_SHARPNESS, Override as GamescopeOverride,
    RESOLUTIONS as GAMESCOPE_RESOLUTIONS, Upscaler as GamescopeUpscaler, WindowMode as GamescopeWindow,
    detect as detect_gamescope, gamescope_display,
};
pub use hud_settings::{
    DEFAULT_TOGGLE_KEY, Detection as HudDetection, FPS_LIMIT_CHOICES, FontSize as HudFontSize, HudSettings,
    INSTALL_HINTS as HUD_INSTALL_HINTS, INSTALL_URL as HUD_INSTALL_URL, Items as HudItems, Position as HudPosition,
    Preset as HudPreset, TextColor as HudColor, detect as detect_hud,
};
pub use interface_settings::{AnimationMode, InterfaceSettings, MAX_SCALE_PERCENT, MIN_SCALE_PERCENT, StartPage};
pub use launch_options::LaunchOptionsState;
pub use notes::MAX_NOTE_CHARS;
pub use overlay_proto::{
    ChatRow, FriendRow, FromHelper, MAX_LINE as OVERLAY_MAX_LINE, MsgRow, Snapshot as OverlaySnapshot, ToHelper,
    snapshot as overlay_snapshot,
};
pub use play::{PlayProblem, RunningGame};
pub use profile::{EditPhase, ProfileEntry, ProfileLoad, ProfileState};
pub use settings::{LibraryChoice, MAX_WORKERS, MIN_WORKERS, Settings};
pub use sr_steam::community::{EditableProfile, MostPlayed, Privacy, ProfileXml, avatar_full_url};
pub use sr_steam::storefront::{AppDetails, Platforms, Price, Screenshot, Section, StoreItem};
pub use storage::MoveState;
pub use sysinfo::system_info;
pub use tools::{
    CompatInfo, CompatState, ReleaseInfo, Source, ToolInfo, ToolInstall, ToolKind, ToolOrigin, ToolPhase, ToolTarget,
};
pub use web_image::WebImage;

use sr_model::{Account, AppId, Game, InstallState, SteamId};
use sr_steam::{auth, local, secrets, store, storefront, webapi};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How the UI is woken from a background thread (`egui::Context::request_repaint`).
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// Events buffered between background tasks and the UI.
const EVENT_QUEUE: usize = 256;

/// The shared connection to Steam's servers (see `cm_hub`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ConnectionStatus {
    /// Not signed in, or demo mode.
    #[default]
    Offline,
    Connecting,
    Online,
    /// The last attempt failed or the connection dropped; a retry is scheduled. Holds the reason.
    Failed(String),
}

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
    /// State of the shared Steam connection.
    pub connection: ConnectionStatus,
    /// Last non-fatal problem, shown in the status bar.
    pub notice: Option<String>,
    pub timings: Timings,
    pub store: StoreState,
    pub downloads: DownloadsState,
    /// Games we started that have not exited yet.
    pub running: HashMap<AppId, RunningGame>,
    /// Set when a launch failed; the UI shows it until dismissed.
    pub play_problem: Option<PlayProblem>,
    /// Games waiting for the Steam client to start before they launch.
    pub starting_steam: Vec<AppId>,
    pub compat: CompatState,
    pub launch_options: LaunchOptionsState,
    /// Per-game "Run in Gamescope" choices (games that follow the global setting are not listed).
    pub gamescope_games: GamescopeGames,
    pub settings: Settings,
    /// Places a game can be installed into.
    pub libraries: Vec<LibraryChoice>,
    /// Free bytes by library path, from `Command::CheckFree` (`None` = unknown). At most [`FREE_KEPT`].
    pub free_space: Vec<(PathBuf, Option<u64>)>,
    pub friends: FriendsState,
    /// The profile page.
    pub profile: ProfileState,
    /// The overlay's per-game notes, by app id.
    pub notes: std::collections::BTreeMap<String, String>,
    /// Favourites, hidden games and collections (ours merged with the official client's).
    pub collections: Collections,
    /// Updates found, install dialog data, storage moves (see `extras`).
    pub extra: ExtraState,
}

/// How many disk-space answers are kept.
const FREE_KEPT: usize = 8;

/// Startup milestones, for the performance log.
#[derive(Debug, Clone, Default)]
pub struct Timings {
    pub cached_library: Option<Duration>,
    pub local_scan: Option<Duration>,
    pub owned_fetch: Option<Duration>,
}

/// A value that is fetched in the background.
#[derive(Debug, Clone, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    /// The value, once it has arrived.
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
}

/// Live search results. `query` is what `items` answers.
#[derive(Debug, Clone, Default)]
pub struct SearchState {
    pub query: String,
    pub items: Vec<StoreItem>,
    pub loading: bool,
    pub error: Option<String>,
}

/// Public store data. Needs no login.
#[derive(Debug, Clone, Default)]
pub struct StoreState {
    pub sections: Load<Arc<Vec<Section>>>,
    pub search: SearchState,
    /// Details of recently opened apps (at most [`DETAILS_KEPT`]).
    pub details: HashMap<u32, Load<Arc<AppDetails>>>,
    details_order: VecDeque<u32>,
}

/// How many app detail pages stay in memory.
const DETAILS_KEPT: usize = 16;
/// Featured rows are fetched again when older than this.
const SECTIONS_MAX_AGE: Duration = Duration::from_secs(10 * 60);

impl State {
    /// The note saved for a game ("" when none).
    pub fn note(&self, app: AppId) -> &str {
        self.notes.get(&app.0.to_string()).map_or("", String::as_str)
    }

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
    /// Start the game with our own launcher.
    Play(AppId),
    /// Stop a game we started (ends its process group).
    Stop(AppId),
    /// Start the game through the official client (`steam://`).
    LaunchViaSteam(AppId),
    /// Dismiss the "could not start" message.
    DismissPlayProblem,
    /// Add a download (install, update or verify) to the queue.
    Download(DownloadRequest),
    /// Queue a verify of an installed game, in its own library.
    Verify(AppId),
    /// Queue an update of an installed game, in its own library.
    Update(AppId),
    PauseDownload(AppId),
    ResumeDownload(AppId),
    /// Cancel an active, queued or paused download.
    CancelDownload(AppId),
    ClearFinishedDownloads,
    /// Updates, install info, library folders, storage moves, notifications (see `ManageCmd`).
    Manage(ManageCmd),
    /// Delete an installed game. `confirmed` must be true for games in a Steam library.
    Uninstall {
        app: AppId,
        confirmed: bool,
    },
    /// Open the game's install folder in the file manager.
    BrowseFiles(AppId),
    /// Ask how much space is free where `PathBuf` is (answer in `State::free_space`).
    CheckFree(PathBuf),
    CompatLoad,
    /// Look up the official client's launch options for a game (answer in `State::launch_options`).
    LoadLaunchOptions(AppId),
    /// Save our launch options text for a game.
    SetLaunchOptions {
        app: AppId,
        text: String,
    },
    /// Our tool override for a game (`None` = use Steam's choice).
    SetCompatTool {
        app: AppId,
        tool: Option<String>,
    },
    /// Fetch the lists of downloadable tools; `true` bypasses the one-hour cache.
    ProtonReleases(bool),
    ProtonInstall {
        source: Source,
        tag: String,
        target: ToolTarget,
    },
    ProtonCancel,
    ProtonRemove(PathBuf),
    SetLibrary(Option<PathBuf>),
    SetWorkers(usize),
    /// Start the official Steam client in the background for games that need it.
    SetStartSteam(bool),
    /// "Write Proton logs": Windows games started with `PROTON_LOG=1` (log files in the data dir's `logs`).
    /// The toggle is in Settings > Compatibility.
    SetProtonLog(bool),
    /// Turn the Shift+Tab in-game overlay on or off.
    SetOverlay(bool),
    /// Save the interface settings (animations, scale, start page, tray).
    SetInterface(InterfaceSettings),
    /// Open a web page in the browser (only a short list of addresses is allowed, see `open`).
    OpenUrl(String),
    /// Save a game's overlay note (an empty text deletes it).
    SetNote(AppId, String),
    /// Save the performance overlay (MangoHud) settings; they apply from the next game launch.
    SetHud(HudSettings),
    /// Save the "Run games in Gamescope" settings; they apply from the next game launch.
    SetGamescope(GamescopeSettings),
    /// A game's own choice: follow the global setting, always, or never.
    SetGameGamescope(AppId, GamescopeOverride),
    /// Favourites, hidden games and collections.
    Collections(CollCmd),
    /// Load the featured rows (no-op while loading or fresh).
    StoreLoad,
    /// Search the store; an empty query clears the results.
    StoreSearch(String),
    StoreDetails(AppId),
    /// Open the app's store page in the system browser.
    OpenStorePage(AppId),
    /// Show the friends panel.
    OpenFriends,
    CloseFriends,
    /// Open (or switch to) the chat with a friend; clears its unread count.
    OpenChat(SteamId),
    /// Close a chat tab.
    CloseChat(SteamId),
    /// Back from a chat to the friends list.
    ShowFriendList,
    SendChat(SteamId, String),
    SetPersonaState(PersonaState),
    /// Send a friend request, accept one, remove a friend or block someone.
    FriendAct(FriendAction, SteamId),
    /// "Add a friend": a friend code, a SteamID or a profile link.
    AddFriend(String),
    /// Open a text channel of a group chat (group id, channel id).
    OpenGroup(u64, u64),
    SendGroupChat(u64, u64, String),
    /// Ask Steam for the group chats again.
    ReloadGroups,
    /// Show someone's profile page (our own SteamID shows our profile).
    ViewProfile(SteamId),
    /// Start editing our profile (asks Steam for a web session and the current values).
    ProfileEditStart,
    ProfileEditSave(EditableProfile),
    ProfileEditCancel,
    /// The window gained or lost focus (desktop notifications only show while it is unfocused).
    WindowFocus(bool),
}

/// Results of background work. `generation` ties a result to the session that
/// asked for it; results from an older generation are dropped.
#[derive(Debug)]
enum Event {
    Qr {
        generation: u64,
        url: String,
    },
    QrScanned {
        generation: u64,
    },
    LoggedIn {
        generation: u64,
        tokens: auth::Tokens,
    },
    Owned {
        generation: u64,
        games: Vec<webapi::OwnedGame>,
        took: Duration,
    },
    /// The owned-games request failed (the cached list stays).
    OwnedFailed {
        generation: u64,
        error: String,
    },
    Hub {
        generation: u64,
        status: ConnectionStatus,
    },
    Installed {
        apps: HashMap<u32, local::InstalledApp>,
        took: Duration,
    },
    DlProgress {
        job: u64,
        progress: sr_steam::content::Progress,
    },
    DlDone {
        job: u64,
        result: downloads::DlResult,
    },
    GameStarted {
        app: AppId,
        pid: u32,
        /// Running inside Gamescope (the overlay then joins its X server).
        gamescope: bool,
    },
    GameExited {
        app: AppId,
        seconds: u64,
        success: bool,
    },
    PlayFailed(PlayProblem),
    /// The game needs the Steam client and we are starting it.
    SteamStarting {
        app: AppId,
    },
    /// The game needs the Steam client but it could not be started (message says why).
    SteamMissing {
        app: AppId,
        message: String,
    },
    Uninstalled {
        app: AppId,
        result: Result<(), String>,
    },
    SteamLaunchOptions {
        app: AppId,
        text: Option<String>,
    },
    CompatInfo {
        generation: u64,
        result: Result<CompatInfo, String>,
        last: bool,
    },
    Releases {
        source: Source,
        result: Result<Vec<sr_steam::proton_releases::Release>, String>,
    },
    ToolProgress {
        job: u64,
        progress: sr_steam::proton_releases::Progress,
    },
    ToolDone {
        job: u64,
        result: Result<String, String>,
    },
    FreeSpace {
        path: PathBuf,
        bytes: Option<u64>,
    },
    LoginFailed {
        generation: u64,
        error: String,
    },
    Notice(String),
    Extra(extras::Extra),
    StoreSections(Result<Vec<Section>, String>),
    StoreSearched {
        generation: u64,
        query: String,
        result: Result<Vec<StoreItem>, String>,
    },
    StoreDetails {
        app: u32,
        result: Result<Option<AppDetails>, String>,
    },
    Friends {
        generation: u64,
        ev: sr_steam::friends::FriendsEvent,
    },
    Profile {
        generation: u64,
        ev: profile::ProfileEvent,
    },
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
    friends: friends::FriendsInner,
    installed: HashMap<u32, local::InstalledApp>,
    /// Play time and last-played as Steam reported them, before our own is added.
    base_play: HashMap<u32, (u32, u64)>,
    play_stats: play::PlayStats,
    /// Games that need the Steam client but run without it: why, for the notice if they exit early.
    steam_missing: HashMap<AppId, String>,
    dl_running: Option<downloads::RunningDl>,
    dl_seq: u64,
    extras: extras::ExtraInner,
    tool_job: Option<tools::ToolJob>,
    tool_seq: u64,
    compat_seq: u64,
    compat_scanning: bool,
    /// Full release records behind `CompatState::available` (needed to install).
    releases: HashMap<&'static str, Vec<sr_steam::proton_releases::Release>>,
    /// Taken in `Drop`, which shuts it down with a time limit.
    rt: std::mem::ManuallyDrop<tokio::runtime::Runtime>,
    /// The shared Steam connection of the signed-in account.
    hub: Option<cm_hub::HubHandle>,
    /// Installed games the account doesn't own (or that the owned list doesn't have yet).
    local_only: HashSet<u32>,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
    wake: Waker,
    generation: Arc<AtomicU64>,
    art: art::ArtLoader,
    web_images: web_image::WebImageLoader,
    /// Newest store search; older answers are dropped.
    search_gen: u64,
    sections_at: Option<Instant>,
}

impl Client {
    pub fn new(wake: Waker) -> anyhow::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let (tx, rx) = mpsc::channel(EVENT_QUEUE);
        let roots = local::steam_roots();
        let settings = Settings::load();
        let mut c = Self {
            state: State {
                screen: Screen::Login(LoginState::default()),
                account: None,
                games: Vec::new(),
                library_rev: 0,
                selected: None,
                refreshing: false,
                connection: ConnectionStatus::Offline,
                notice: None,
                timings: Timings::default(),
                store: StoreState::default(),
                downloads: DownloadsState::default(),
                running: HashMap::new(),
                play_problem: None,
                starting_steam: Vec::new(),
                compat: CompatState::default(),
                launch_options: LaunchOptionsState {
                    ours: sr_steam::launch_opts::LaunchOptions::load().apps,
                    ..Default::default()
                },
                gamescope_games: GamescopeGames::load(),
                libraries: settings::library_choices(&settings),
                settings,
                free_space: Vec::new(),
                friends: FriendsState::default(),
                profile: ProfileState::default(),
                notes: notes::Notes::load().apps,
                collections: Collections::load(),
                extra: ExtraState::default(),
            },
            session: None,
            friends: friends::FriendsInner::default(),
            installed: HashMap::new(),
            base_play: HashMap::new(),
            play_stats: play::PlayStats::load(),
            steam_missing: HashMap::new(),
            dl_running: None,
            dl_seq: 0,
            extras: Default::default(),
            tool_job: None,
            tool_seq: 0,
            compat_seq: 0,
            compat_scanning: false,
            releases: HashMap::new(),
            art: art::ArtLoader::new(roots, wake.clone()),
            web_images: web_image::WebImageLoader::new(wake.clone()),
            search_gen: 0,
            sections_at: None,
            rt: std::mem::ManuallyDrop::new(rt),
            hub: None,
            local_only: HashSet::new(),
            tx,
            rx,
            wake,
            generation: Arc::new(AtomicU64::new(0)),
        };
        c.spawn_local_scan();
        match std::env::var("KETTLE_DEMO").ok().and_then(|v| v.parse::<usize>().ok()) {
            Some(n) => {
                c.start_demo(n);
                c.demo_running();
            }
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
            Command::Play(app) => self.play(app),
            Command::Stop(app) => self.stop_game(app),
            Command::LaunchViaSteam(app) => self.notice_on_err(launch::play(app)),
            Command::DismissPlayProblem => self.state.play_problem = None,
            Command::Download(req) => self.dl_enqueue(req),
            Command::Verify(app) => self.dl_verify(app),
            Command::Update(app) => self.dl_update(app),
            Command::PauseDownload(app) => self.dl_pause(app),
            Command::ResumeDownload(app) => self.dl_resume(app),
            Command::CancelDownload(app) => self.dl_cancel(app),
            Command::ClearFinishedDownloads => self.dl_clear_finished(),
            Command::Manage(m) => self.manage(m),
            Command::Uninstall { app, confirmed } => self.uninstall(app, confirmed),
            Command::BrowseFiles(app) => self.browse_files(app),
            Command::CheckFree(path) => self.check_free(path),
            Command::CompatLoad => self.compat_load(),
            Command::LoadLaunchOptions(app) => self.load_launch_options(app),
            Command::SetLaunchOptions { app, text } => self.set_launch_options(app, text),
            Command::SetCompatTool { app, tool } => self.set_compat_tool(app.0, tool),
            Command::ProtonReleases(refresh) => self.proton_releases(refresh),
            Command::ProtonInstall { source, tag, target } => self.proton_install(source, tag, target),
            Command::ProtonCancel => self.proton_cancel(),
            Command::ProtonRemove(dir) => self.proton_remove(dir),
            c @ (Command::SetLibrary(_)
            | Command::SetWorkers(_)
            | Command::SetStartSteam(_)
            | Command::SetProtonLog(_)
            | Command::SetOverlay(_)
            | Command::SetInterface(_)) => settings::handle(self, c),
            Command::OpenUrl(u) => self.notice_on_err(open::url(&u)),
            Command::SetNote(app, text) => self.set_note(app, text),
            Command::SetHud(h) => self.set_hud(h),
            Command::SetGamescope(g) => self.set_gamescope(g),
            Command::SetGameGamescope(app, o) => self.set_game_gamescope(app, o),
            Command::Collections(c) => self.collections_cmd(c),
            Command::StoreLoad => self.store_load(),
            Command::StoreSearch(q) => self.store_search(q),
            Command::StoreDetails(app) => self.store_details(app),
            Command::OpenFriends => self.friends_open(true),
            Command::CloseFriends => self.friends_open(false),
            Command::OpenChat(id) => self.open_chat(id),
            Command::CloseChat(id) => self.close_chat(id),
            Command::ShowFriendList => self.show_friend_list(),
            Command::SendChat(id, text) => self.send_chat(id, text),
            Command::SetPersonaState(s) => self.set_persona_state(s),
            Command::FriendAct(a, id) => self.friend_action(a, id),
            Command::AddFriend(input) => self.add_friend_input(&input),
            Command::OpenGroup(g, r) => self.open_group(g, r),
            Command::SendGroupChat(g, r, text) => self.send_group_chat(g, r, text),
            Command::ReloadGroups => self.load_groups(),
            Command::ViewProfile(id) => self.view_profile(id),
            Command::ProfileEditStart => self.profile_edit_start(),
            Command::ProfileEditSave(p) => self.profile_edit_save(p),
            Command::ProfileEditCancel => self.profile_edit_cancel(),
            Command::WindowFocus(on) => self.state.friends.window_unfocused = !on,
            Command::OpenStorePage(app) => {
                self.notice_on_err(open::url(&format!("https://store.steampowered.com/app/{app}")));
            }
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

    /// Ask for a store image by URL; cheap to call every frame for visible ones.
    pub fn request_web_image(&self, url: &str) {
        self.web_images.request(url);
    }

    pub fn forget_web_image(&self, url: &str) {
        self.web_images.forget(url);
    }

    pub fn take_web_images(&self) -> Vec<WebImage> {
        self.web_images.take()
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
                if self.state.settings.keep_signed_in
                    && let Err(e) = secrets::save_refresh_token(&account.account_name, &tokens.refresh_token)
                {
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
                self.local_only.clear();
                self.state.games = list;
                self.capture_base_play();
                self.merge_installed();
            }
            Event::OwnedFailed { generation, error } if generation == current => {
                self.state.refreshing = false;
                self.state.notice = Some(format!("Couldn't load your games from Steam: {error}"));
            }
            Event::Hub { generation, status } if generation == current => self.apply_hub(status),
            Event::Installed { apps, took } => {
                self.state.timings.local_scan = Some(took);
                self.installed = apps;
                self.merge_installed();
                self.run_pending_play(true);
            }
            Event::DlProgress { job, progress } => self.dl_progress(job, progress),
            Event::DlDone { job, result } => self.dl_done(job, result),
            Event::Extra(e) => self.apply_extra(e),
            Event::GameStarted { app, pid, gamescope } => self.game_started(app, pid, gamescope),
            Event::GameExited { app, seconds, success } => self.game_exited(app, seconds, success),
            Event::PlayFailed(problem) => self.play_failed(problem),
            Event::SteamStarting { app } => self.steam_starting(app),
            Event::SteamMissing { app, message } => self.steam_missing(app, message),
            Event::Uninstalled { app, result } => {
                self.state.notice = Some(match result {
                    Ok(()) => format!("{} was uninstalled.", self.state.game(app).map_or("The game", |g| &g.name)),
                    Err(e) => format!("Uninstall failed: {e}"),
                });
                self.spawn_local_scan();
            }
            Event::SteamLaunchOptions { app, text } => self.steam_launch_options(app, text),
            Event::CompatInfo { generation, result, last } => self.apply_compat_info(generation, result, last),
            Event::Releases { source, result } => self.apply_releases(source, result),
            Event::ToolProgress { job, progress } => self.tool_progress(job, progress),
            Event::ToolDone { job, result } => self.tool_done(job, result),
            Event::FreeSpace { path, bytes } => {
                let f = &mut self.state.free_space;
                f.retain(|(p, _)| *p != path);
                f.push((path, bytes));
                if f.len() > FREE_KEPT {
                    f.remove(0);
                }
            }
            Event::Notice(msg) => self.state.notice = Some(msg),
            Event::StoreSections(result) => self.apply_sections(result),
            Event::StoreSearched { generation, query, result } if generation == self.search_gen => {
                let s = &mut self.state.store.search;
                s.loading = false;
                s.query = query;
                match result {
                    Ok(items) => (s.items, s.error) = (items, None),
                    Err(e) => (s.items, s.error) = (Vec::new(), Some(e)),
                }
            }
            Event::StoreDetails { app, result } => self.apply_details(app, result),
            Event::Friends { generation, ev } if generation == current => self.apply_friends(ev),
            Event::Profile { generation, ev } if generation == current => self.apply_profile(ev),
            // Stale result from an older generation.
            _ => {}
        }
    }

    fn apply_sections(&mut self, result: Result<Vec<Section>, String>) {
        self.state.store.sections = match result {
            Ok(v) => {
                self.sections_at = Some(Instant::now());
                Load::Ready(Arc::new(v))
            }
            Err(e) => Load::Failed(e),
        };
    }

    fn apply_details(&mut self, app: u32, result: Result<Option<AppDetails>, String>) {
        let st = &mut self.state.store;
        st.details.insert(
            app,
            match result {
                Ok(Some(d)) => Load::Ready(Arc::new(d)),
                Ok(None) => Load::Failed("The store has no page for this app.".into()),
                Err(e) => Load::Failed(e),
            },
        );
    }

    /// Recompute install state and notify views that `games` changed.
    fn merge_installed(&mut self) {
        self.state.library_rev += 1;
        self.reload_steam_collections();
        self.add_local_games();
        self.art.set_icon_hashes(self.state.games.iter().map(|g| (g.app_id, g.icon_hash.clone())).collect());
        for g in &mut self.state.games {
            // Steam's numbers plus the time we measured ourselves.
            if let Some(&(minutes, last)) = self.base_play.get(&g.app_id.0) {
                let (secs, last_run) = self.play_stats.get(g.app_id);
                g.playtime_minutes = minutes.saturating_add((secs / 60) as u32);
                g.last_played = last.max(last_run);
            }
            g.install = match self.installed.get(&g.app_id.0) {
                Some(a) if a.fully_installed() || a.state_flags & 2 != 0 => {
                    let path = a.install_dir.display().to_string();
                    if a.state_flags & 2 != 0 || self.state.extra.updates.contains(&a.appid) {
                        InstallState::UpdateRequired { size_bytes: a.size_on_disk, path }
                    } else {
                        InstallState::Installed { size_bytes: a.size_on_disk, path }
                    }
                }
                _ => InstallState::NotInstalled,
            };
        }
    }

    /// Show games installed on this machine even when the owned list is empty or can't be loaded (no network,
    /// not signed in to Steam yet, or a game from a family share). They sit in the list until the owned
    /// list has them.
    fn add_local_games(&mut self) {
        let old = std::mem::take(&mut self.local_only);
        self.state.games.retain(|g| !old.contains(&g.app_id.0));
        let extra = local_games(&self.installed, &self.state.games);
        self.local_only = extra.iter().map(|g| g.app_id.0).collect();
        let changed = !extra.is_empty() || !old.is_empty();
        self.state.games.extend(extra);
        if changed {
            self.state.games.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));
        }
    }

    fn apply_hub(&mut self, status: ConnectionStatus) {
        self.hub_status_changed(matches!(status, ConnectionStatus::Online));
        match &status {
            ConnectionStatus::Connecting => self.state.friends.status = FriendsStatus::Connecting,
            ConnectionStatus::Failed(why) => self.state.friends.status = FriendsStatus::Disconnected(why.clone()),
            _ => {}
        }
        self.state.connection = status;
    }

    /// Remember the play time and last-played Steam reported, so [`Self::merge_installed`] can add ours on top
    /// every time without counting it twice.
    fn capture_base_play(&mut self) {
        self.base_play = self.state.games.iter().map(|g| (g.app_id.0, (g.playtime_minutes, g.last_played))).collect();
    }

    fn check_free(&mut self, path: PathBuf) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let bytes = downloads::free_space(&path);
            let _ = tx.blocking_send(Event::FreeSpace { path, bytes });
            wake();
        });
    }

    fn notice_on_err(&mut self, r: anyhow::Result<()>) {
        if let Err(e) = r {
            self.state.notice = Some(format!("{e:#}"));
        }
    }

    // ---- store ---------------------------------------------------------

    /// Run `job` on a blocking worker, send its event and wake the UI.
    fn spawn_store<T: Send + 'static>(
        &self,
        job: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
        event: impl FnOnce(Result<T, String>) -> Event + Send + 'static,
    ) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let _ = tx.blocking_send(event(job().map_err(|e| format!("{e:#}"))));
            wake();
        });
    }

    fn store_load(&mut self) {
        let fresh = self.sections_at.is_some_and(|t| t.elapsed() < SECTIONS_MAX_AGE);
        match self.state.store.sections {
            Load::Loading => return,
            Load::Ready(_) if fresh => return,
            _ => {}
        }
        // Keep showing the old rows while a refresh runs.
        if !matches!(self.state.store.sections, Load::Ready(_)) {
            self.state.store.sections = Load::Loading;
        }
        self.spawn_store(storefront::featured_sections, Event::StoreSections);
    }

    fn store_search(&mut self, query: String) {
        self.search_gen += 1;
        let generation = self.search_gen;
        let query = query.trim().to_string();
        let s = &mut self.state.store.search;
        s.error = None;
        if query.is_empty() {
            *s = SearchState::default();
            return;
        }
        s.loading = true;
        let q = query.clone();
        self.spawn_store(
            move || storefront::search(&q),
            move |result| Event::StoreSearched { generation, query, result },
        );
    }

    fn store_details(&mut self, app: AppId) {
        let st = &mut self.state.store;
        if matches!(st.details.get(&app.0), Some(Load::Loading | Load::Ready(_))) {
            return;
        }
        st.details.insert(app.0, Load::Loading);
        st.details_order.retain(|a| *a != app.0);
        st.details_order.push_back(app.0);
        while st.details_order.len() > DETAILS_KEPT {
            if let Some(old) = st.details_order.pop_front() {
                st.details.remove(&old);
            }
        }
        let id = app.0;
        self.spawn_store(move || storefront::app_details(id), move |result| Event::StoreDetails { app: id, result });
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
            self.local_only.clear();
            self.state.games = cached;
            self.capture_base_play();
            self.state.timings.cached_library = Some(t.elapsed());
            self.merge_installed();
        }
        self.state.account = Some(account.clone());
        self.state.screen = Screen::Library;
        self.session = Some(Session { account, refresh_token });
        self.start_hub();
        self.spawn_refresh();
        self.start_friends();
    }

    fn start_demo(&mut self, n: usize) {
        let t = Instant::now();
        self.local_only.clear();
        self.state.games = demo::games(n);
        self.capture_base_play();
        self.state.timings.cached_library = Some(t.elapsed());
        self.state.account = Some(Account { account_name: "demo".into(), steam_id: SteamId(0) });
        self.state.screen = Screen::Library;
        self.merge_installed();
        let ids: Vec<u32> = self.state.games.iter().map(|g| g.app_id.0).collect();
        self.state.collections.set_demo(&ids);
        if std::env::var_os("KETTLE_HUB_STALL").is_some() {
            // Test aid: a signed-in look-alike whose Steam connection never completes (see `cm_hub`).
            self.session =
                Some(Session { account: self.state.account.clone().expect("set above"), refresh_token: String::new() });
            self.start_hub();
            self.spawn_refresh();
            self.start_friends();
        } else {
            self.start_friends_demo();
        }
    }

    /// `KETTLE_DEMO_RUNNING=<appid>`: show a demo game as running, without starting any process.
    fn demo_running(&mut self) {
        let app = std::env::var("KETTLE_DEMO_RUNNING").ok().and_then(|v| v.parse::<u32>().ok());
        if let Some(app) = app {
            // pid 0 is never signalled (see `stop_game`).
            self.state.running.insert(AppId(app), RunningGame { pid: 0, started: Instant::now(), gamescope: false });
        }
    }

    fn logout(&mut self) {
        // Downloads belong to the account that asked for them.
        self.dl_cancel_all();
        if let Some(s) = self.session.take() {
            self.notice_on_err(secrets::delete_refresh_token(&s.account.account_name));
            forget_account(&s.account);
            let _ = std::fs::remove_file(library_cache_path(s.account.steam_id));
        }
        self.bump_generation();
        self.hub = None;
        self.state.connection = ConnectionStatus::Offline;
        self.reset_friends();
        self.state.profile = ProfileState::default();
        self.state.account = None;
        self.state.games.clear();
        self.state.extra.updates.clear();
        self.state.extra.update_check = UpdateCheck::Idle;
        self.local_only.clear();
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
            let mut session = match auth::begin_qr(auth::DEVICE_NAME) {
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

    /// Open the shared Steam connection for the current session.
    pub(crate) fn start_hub(&mut self) {
        let Some(s) = &self.session else { return };
        let generation = self.generation.load(Ordering::Relaxed);
        self.state.connection = ConnectionStatus::Connecting;
        self.hub = Some(cm_hub::spawn(
            self.rt.handle(),
            s.account.account_name.clone(),
            cm_hub::TokenCopy(s.refresh_token.clone()),
            generation,
            self.generation.clone(),
            self.tx.clone(),
            self.wake.clone(),
        ));
    }

    /// Load the owned games over the shared connection. If a request fails, it is tried again on the next
    /// connection (a reconnect), and the cached list stays until then.
    fn spawn_refresh(&mut self) {
        let (Some(s), Some(hub)) = (&self.session, &self.hub) else { return };
        self.state.refreshing = true;
        let generation = self.generation.load(Ordering::Relaxed);
        let (tx, wake, mut hub) = (self.tx.clone(), self.wake.clone(), hub.clone());
        let steamid = s.account.steam_id.0;
        self.rt.spawn(async move {
            let mut last = 0;
            while let Some((id, cm)) = hub.wait_online(last).await {
                last = id;
                let t = Instant::now();
                let ev = match sr_steam::cm::owned_games(&cm.conn, steamid).await {
                    Ok(games) => Event::Owned { generation, games, took: t.elapsed() },
                    Err(e) => Event::OwnedFailed { generation, error: format!("{e:#}") },
                };
                let done = matches!(ev, Event::Owned { .. });
                let _ = tx.send(ev).await;
                wake();
                if done {
                    break;
                }
            }
        });
    }

    fn spawn_local_scan(&self) {
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let extra = self.extra_libraries();
        self.rt.spawn_blocking(move || {
            let t = Instant::now();
            let apps = local::installed_apps_with(&extra);
            let _ = tx.blocking_send(Event::Installed { apps, took: t.elapsed() });
            wake();
        });
    }
}

/// Installed games (not tools) that `have` does not list.
fn local_games(installed: &HashMap<u32, local::InstalledApp>, have: &[Game]) -> Vec<Game> {
    let have: HashSet<u32> = have.iter().map(|g| g.app_id.0).collect();
    let mut out: Vec<Game> = installed
        .values()
        .filter(|a| (a.fully_installed() || a.state_flags & 2 != 0) && !have.contains(&a.appid))
        .filter(|a| !local::is_tool_app(a.appid, &a.name))
        .map(|a| {
            let name = if a.name.is_empty() { format!("App {}", a.appid) } else { a.name.clone() };
            let mut g = Game::new(AppId(a.appid), name);
            g.last_played = a.last_updated;
            g
        })
        .collect();
    out.sort_by_key(|g| g.app_id.0);
    out
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

impl Drop for Client {
    /// Closing the window must be quick: stop the downloads (they resume next time), end the sessions and
    /// give the background tasks a moment instead of waiting for them.
    fn drop(&mut self) {
        self.bump_generation();
        if let Some(run) = &self.dl_running {
            run.ctl.store(1, Ordering::SeqCst);
        }
        // SAFETY: `rt` is not used after this.
        let rt = unsafe { std::mem::ManuallyDrop::take(&mut self.rt) };
        rt.shutdown_timeout(SHUTDOWN_WAIT);
    }
}

/// How long closing waits for background tasks that are in the middle of a blocking call.
const SHUTDOWN_WAIT: Duration = Duration::from_millis(700);

#[cfg(test)]
mod local_games_tests {
    use super::*;

    fn app(id: u32, name: &str, flags: u32) -> (u32, local::InstalledApp) {
        let a = local::InstalledApp {
            appid: id,
            name: name.into(),
            install_dir: PathBuf::from("/x"),
            size_on_disk: 1,
            state_flags: flags,
            build_id: 1,
            last_updated: 5,
        };
        (id, a)
    }

    #[test]
    fn installed_games_show_without_an_owned_list() {
        let installed: HashMap<u32, local::InstalledApp> = [
            app(10, "Alpha", 4),
            app(20, "Beta", 4),
            app(30, "Half downloaded", 1026 & !2 & !4),
            app(228980, "Steamworks Common Redistributables", 4),
            app(40, "", 4),
        ]
        .into();
        let owned = vec![Game::new(AppId(20), "Beta".into())];
        let out = local_games(&installed, &owned);
        let names: Vec<_> = out.iter().map(|g| (g.app_id.0, g.name.as_str())).collect();
        assert_eq!(names, [(10, "Alpha"), (40, "App 40")]);
        assert_eq!(local_games(&installed, &[]).len(), 3);
    }
}
