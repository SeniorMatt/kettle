//! Updates, install info, library folders, storage moves and the other commands of the Downloads, Storage,
//! Notifications and Account settings pages. The UI sends [`ManageCmd`]s; results come back as [`Extra`]
//! events and end up in [`ExtraState`].

use super::download_settings::{DownloadSettings, NotifySettings, SteamUpdatePolicy, UpdateBehavior, local_hour};
use super::storage::{self, MoveState};
use super::{Client, Event, Load, OsChoice, downloads};
use sr_model::AppId;
use sr_steam::content::estimate;
use sr_steam::{content, local, secrets, store};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub use sr_steam::content::estimate::{DlcInfo, InstallInfo, with_margin};

/// How often installed games are checked for updates while signed in.
const UPDATE_EVERY: Duration = Duration::from_secs(45 * 60);
/// A check this young is not repeated when the connection comes back.
const RECHECK_AFTER_SECS: u64 = 600;
/// How many install-dialog answers are kept.
const INFO_KEPT: usize = 6;
/// How long the install dialog's size lookup waits for the Steam connection.
const INFO_WAIT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq)]
pub enum ManageCmd {
    /// Replace the download settings (speed limit, schedule, update policy, added libraries).
    SetDownloads(DownloadSettings),
    SetNotifications(NotifySettings),
    /// Keep the sign-in token in the keyring (off: sign in with a QR code on the next start).
    SetKeepSignedIn(bool),
    SetUpdateBehavior(AppId, UpdateBehavior),
    /// Look for updates of installed games now.
    CheckUpdates,
    /// The answer to "the official client uses this folder": remember `policy` and, when `update`, update now.
    SteamUpdateAnswer {
        app: AppId,
        policy: SteamUpdatePolicy,
        update: bool,
    },
    DismissSteamUpdatePrompt,
    /// Play although an update is waiting.
    PlayAnyway(AppId),
    DismissUpdatePrompt,
    /// Start the game once the installed games have been scanned (`kettle --play <appid>`).
    PlayWhenReady(u32),
    /// Look up size, DLC and languages for the install dialog.
    LoadInstallInfo {
        app: AppId,
        os: OsChoice,
        language: String,
    },
    /// Move a queued download up (negative) or down in the queue.
    MoveDownload(AppId, i32),
    PrioritizeDownload(AppId),
    PauseAllDownloads,
    ResumeAllDownloads,
    AddLibrary(PathBuf),
    /// Forget a library folder of ours (only when no game is in it; the files are not deleted).
    RemoveLibrary(PathBuf),
    /// Copy a game to another library and delete the old copy. `confirmed` is needed when either folder
    /// belongs to the official client.
    MoveGame {
        app: AppId,
        to: PathBuf,
        confirmed: bool,
    },
    CancelMove,
    DismissMove,
    /// Write a desktop icon and/or an application menu entry for a game.
    CreateShortcuts {
        app: AppId,
        desktop: bool,
        menu: bool,
    },
    /// Delete the cache folder (art, library cache, release lists). Nothing of the games is touched.
    ClearCache,
    /// Show a test notification.
    TestNotification,
}

/// Background results for this module.
#[derive(Debug)]
pub(crate) enum Extra {
    UpdatesFound { generation: u64, apps: Vec<u32> },
    UpdateCheckFailed { generation: u64, error: String },
    UpdateTick { generation: u64 },
    InstallInfo { app: AppId, result: Result<InstallInfo, String> },
    MoveProgress { app: AppId, done: u64 },
    MoveTotal { app: AppId, total: u64 },
    MoveDone { app: AppId, result: Result<(), String> },
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum UpdateCheck {
    #[default]
    Idle,
    Running,
    /// Unix time of the last finished check and how many games have an update.
    Done {
        at: u64,
        found: usize,
    },
    Failed(String),
}

/// What the UI reads.
#[derive(Debug, Clone, Default)]
pub struct ExtraState {
    /// Installed games that have an update on Steam.
    pub updates: BTreeSet<u32>,
    pub update_check: UpdateCheck,
    /// Install dialog data by app id (at most a few).
    pub install_info: HashMap<u32, Load<Arc<InstallInfo>>>,
    /// The user pressed Play on a game with an update: "Update now / Play anyway".
    pub update_prompt: Option<AppId>,
    /// An update of a game in the official client's library needs the user's say-so.
    pub steam_update_prompt: Option<AppId>,
    pub moving: Option<MoveState>,
}

/// Private bookkeeping.
#[derive(Default)]
pub(crate) struct ExtraInner {
    /// The session generation the update ticker runs for.
    ticker_gen: Option<u64>,
    pending_play: Option<u32>,
    /// Games the user chose to play although an update waits (consumed by the next Play).
    bypass: HashSet<u32>,
    move_cancel: Option<Arc<AtomicBool>>,
}

impl Client {
    pub(crate) fn manage(&mut self, cmd: ManageCmd) {
        match cmd {
            ManageCmd::SetDownloads(d) => self.set_downloads(d),
            ManageCmd::SetNotifications(n) => {
                self.state.settings.notifications = n;
                self.save_settings();
            }
            ManageCmd::SetKeepSignedIn(on) => self.set_keep_signed_in(on),
            ManageCmd::SetUpdateBehavior(app, b) => {
                let map = &mut self.state.settings.downloads.behavior;
                if b == UpdateBehavior::default() {
                    map.remove(&app.0.to_string());
                } else {
                    map.insert(app.0.to_string(), b);
                }
                self.save_settings();
            }
            ManageCmd::CheckUpdates => self.check_updates(),
            ManageCmd::SteamUpdateAnswer { app, policy, update } => {
                self.state.extra.steam_update_prompt = None;
                if policy != SteamUpdatePolicy::Ask {
                    self.state.settings.downloads.steam_policy = policy;
                    self.save_settings();
                }
                if update {
                    self.dl_update_confirmed(app, false);
                }
            }
            ManageCmd::DismissSteamUpdatePrompt => self.state.extra.steam_update_prompt = None,
            ManageCmd::PlayAnyway(app) => {
                self.state.extra.update_prompt = None;
                self.extras.bypass.insert(app.0);
                self.play(app);
            }
            ManageCmd::DismissUpdatePrompt => self.state.extra.update_prompt = None,
            ManageCmd::PlayWhenReady(id) => {
                self.extras.pending_play = Some(id);
                self.run_pending_play(false);
            }
            ManageCmd::LoadInstallInfo { app, os, language } => self.load_install_info(app, os, language),
            ManageCmd::MoveDownload(app, delta) => self.dl_move(app, delta),
            ManageCmd::PrioritizeDownload(app) => self.dl_prioritize(app),
            ManageCmd::PauseAllDownloads => self.dl_pause_all(),
            ManageCmd::ResumeAllDownloads => self.dl_resume_all(),
            ManageCmd::AddLibrary(p) => self.add_library(p),
            ManageCmd::RemoveLibrary(p) => self.remove_library(p),
            ManageCmd::MoveGame { app, to, confirmed } => self.move_game(app, to, confirmed),
            ManageCmd::CancelMove => {
                if let Some(c) = &self.extras.move_cancel {
                    c.store(true, Ordering::SeqCst);
                }
            }
            ManageCmd::DismissMove => {
                if self.state.extra.moving.as_ref().is_some_and(|m| m.result.is_some()) {
                    self.state.extra.moving = None;
                }
            }
            ManageCmd::CreateShortcuts { app, desktop, menu } => {
                let name = self.state.game(app).map(|g| g.name.clone()).unwrap_or_default();
                self.make_shortcuts(app, name, desktop, menu);
            }
            ManageCmd::ClearCache => self.spawn_store(
                || {
                    let dir = store::cache_dir();
                    let mut removed = 0usize;
                    for e in std::fs::read_dir(&dir)?.flatten() {
                        let p = e.path();
                        let ok = if e.file_type().is_ok_and(|t| t.is_dir()) {
                            std::fs::remove_dir_all(&p).is_ok()
                        } else {
                            std::fs::remove_file(&p).is_ok()
                        };
                        removed += usize::from(ok);
                    }
                    Ok(removed)
                },
                |r| {
                    Event::Notice(match r {
                        Ok(n) => format!("Cache cleared ({n} items). Pictures load again when needed."),
                        Err(e) => format!("Couldn't clear the cache: {e}"),
                    })
                },
            ),
            ManageCmd::TestNotification => {
                super::notify::show("Kettle", "Notifications work.", self.state.settings.notifications.sound);
            }
        }
    }

    fn set_downloads(&mut self, mut d: DownloadSettings) {
        d.limit_kbps = d.limit_kbps.min(10_000_000);
        d.schedule_from %= 24;
        d.schedule_to %= 24;
        self.state.settings.downloads = d;
        self.save_settings();
        self.apply_bandwidth();
    }

    fn set_keep_signed_in(&mut self, on: bool) {
        self.state.settings.keep_signed_in = on;
        self.save_settings();
        let Some(s) = &self.session else { return };
        let r = if on {
            secrets::save_refresh_token(&s.account.account_name, &s.refresh_token)
        } else {
            secrets::delete_refresh_token(&s.account.account_name)
        };
        if let Err(e) = r {
            self.state.notice = Some(format!("Couldn't update the keyring: {e:#}"));
        }
    }

    // ---- libraries ---------------------------------------------------------

    fn add_library(&mut self, path: PathBuf) {
        if !path.is_absolute() || path.parent().is_none() {
            self.state.notice = Some("Pick an absolute folder path for the library.".into());
            return;
        }
        // A library is a folder with `steamapps` in it, the shape the official client reads too.
        if let Err(e) = std::fs::create_dir_all(path.join("steamapps")) {
            self.state.notice = Some(format!("Couldn't create the library folder: {e}"));
            return;
        }
        let libs = &mut self.state.settings.downloads.libraries;
        if !libs.contains(&path) {
            libs.push(path);
        }
        self.settings_changed();
    }

    fn remove_library(&mut self, path: PathBuf) {
        let in_use =
            self.installed.values().any(|a| downloads::library_of(&a.install_dir).as_deref() == Some(path.as_path()));
        if in_use {
            self.state.notice = Some("There are games in this library. Move or uninstall them first (Storage).".into());
            return;
        }
        let s = &mut self.state.settings;
        s.downloads.libraries.retain(|p| *p != path);
        if s.library.as_ref() == Some(&path) {
            s.library = None;
        }
        self.settings_changed();
    }

    // ---- moving a game -----------------------------------------------------

    fn move_game(&mut self, app: AppId, to: PathBuf, confirmed: bool) {
        if self.state.extra.moving.as_ref().is_some_and(|m| m.result.is_none()) {
            self.state.notice = Some("Another move is running.".into());
            return;
        }
        if self.state.running.contains_key(&app) || self.state.downloads.status(app) != super::DownloadStatus::None {
            self.state.notice = Some("Stop the game or its download first.".into());
            return;
        }
        let Some(inst) = self.installed.get(&app.0).cloned() else { return };
        if !self.state.libraries.iter().any(|l| l.path == to) {
            self.state.notice = Some("That folder is not one of your libraries.".into());
            return;
        }
        let plan = match storage::plan_move(&inst, &to) {
            Ok(p) => p,
            Err(e) => {
                self.state.notice = Some(format!("Can't move: {e:#}"));
                return;
            }
        };
        if !confirmed && !(self.is_our_library(&plan.from.library) && self.is_our_library(&to)) {
            self.state.notice = Some("Moving to or from a Steam library needs your confirmation.".into());
            return;
        }
        let name = self.state.game(app).map_or_else(|| inst.name.clone(), |g| g.name.clone());
        self.state.extra.moving = Some(MoveState { app, name, to, done: 0, total: 0, result: None });
        let cancel = Arc::new(AtomicBool::new(false));
        self.extras.move_cancel = Some(cancel.clone());
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let send = |ev: Extra| {
                let _ = tx.blocking_send(Event::Extra(ev));
                wake();
            };
            send(Extra::MoveTotal { app, total: storage::move_size(&plan) });
            let mut last = std::time::Instant::now();
            let result = storage::run_move(&plan, &cancel, &mut |done| {
                if last.elapsed() >= Duration::from_millis(250) {
                    last = std::time::Instant::now();
                    send(Extra::MoveProgress { app, done });
                }
            });
            send(Extra::MoveDone { app, result: result.map_err(|e| format!("{e:#}")) });
        });
    }

    // ---- updates -----------------------------------------------------------

    /// Called when the shared connection changes state: on "online", start the update ticker and check once.
    pub(crate) fn hub_status_changed(&mut self, online: bool) {
        if !online || self.session.is_none() {
            return;
        }
        let generation = self.generation.load(Ordering::Relaxed);
        if self.extras.ticker_gen != Some(generation) {
            self.extras.ticker_gen = Some(generation);
            let (tx, wake, current) = (self.tx.clone(), self.wake.clone(), self.generation.clone());
            self.rt.spawn(async move {
                loop {
                    tokio::time::sleep(UPDATE_EVERY).await;
                    if current.load(Ordering::Relaxed) != generation {
                        break;
                    }
                    let _ = tx.send(Event::Extra(Extra::UpdateTick { generation })).await;
                    wake();
                }
            });
        }
        let fresh = matches!(self.state.extra.update_check, UpdateCheck::Done { at, .. } if downloads::now_unix().saturating_sub(at) < RECHECK_AFTER_SECS);
        if !fresh {
            self.check_updates();
        }
    }

    /// Ask Steam whether installed games have updates (needs the shared connection).
    pub(crate) fn check_updates(&mut self) {
        if self.state.extra.update_check == UpdateCheck::Running || self.session.is_none() {
            return;
        }
        let Some(hub) = self.hub.clone() else { return };
        let apps: Vec<(u32, PathBuf)> = self
            .installed
            .values()
            .filter(|a| a.fully_installed() && !local::is_tool_app(a.appid, &a.name))
            .filter_map(|a| downloads::library_of(&a.install_dir).map(|l| (a.appid, l)))
            .collect();
        if apps.is_empty() {
            return;
        }
        self.state.extra.update_check = UpdateCheck::Running;
        let generation = self.generation.load(Ordering::Relaxed);
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn(async move {
            // The hub announces "online" a moment before its connection can be fetched: wait for it.
            let mut cm = hub.online_now();
            for _ in 0..20 {
                if cm.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                cm = hub.online_now();
            }
            let ev = match cm {
                Some(cm) => match content::updates::find_updates(&cm.conn, &apps).await {
                    Ok(apps) => Extra::UpdatesFound { generation, apps },
                    Err(e) => Extra::UpdateCheckFailed { generation, error: format!("{e:#}") },
                },
                None => Extra::UpdateCheckFailed { generation, error: "Not connected to Steam.".into() },
            };
            let _ = tx.send(Event::Extra(ev)).await;
            wake();
        });
    }

    /// Play on a game with an update waiting: stop and ask. The Play command calls this first.
    pub(crate) fn update_gate(&mut self, app: AppId) -> bool {
        if !self.state.extra.updates.contains(&app.0) || self.extras.bypass.remove(&app.0) {
            return false;
        }
        // Games in the official client's folders that we may not update: nothing to offer, just play.
        let theirs_and_never =
            self.installed.get(&app.0).and_then(|a| downloads::library_of(&a.install_dir)).is_some_and(|l| {
                !self.is_our_library(&l) && self.state.settings.downloads.steam_policy == SteamUpdatePolicy::Never
            });
        if theirs_and_never {
            return false;
        }
        self.state.extra.update_prompt = Some(app);
        true
    }

    /// After a scan: start the game asked for with `--play` once it is known.
    pub(crate) fn run_pending_play(&mut self, scanned: bool) {
        let Some(id) = self.extras.pending_play else { return };
        if self.installed.contains_key(&id) {
            self.extras.pending_play = None;
            self.play(AppId(id));
        } else if scanned {
            self.extras.pending_play = None;
            self.state.notice = Some(format!("App {id} is not installed."));
        }
    }

    // ---- install info ------------------------------------------------------

    fn load_install_info(&mut self, app: AppId, os: OsChoice, language: String) {
        let info = &mut self.state.extra.install_info;
        if info.len() >= INFO_KEPT {
            let drop_key = info.keys().copied().find(|k| *k != app.0);
            if let Some(k) = drop_key {
                info.remove(&k);
            }
        }
        info.insert(app.0, Load::Loading);
        let language = if language.is_empty() {
            self.state.settings.downloads.language_or_default().to_string()
        } else {
            language
        };
        let hub = self.hub.clone().filter(|_| self.session.is_some());
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn(async move {
            let result = async {
                match hub {
                    Some(hub) => {
                        let start = std::time::Instant::now();
                        let cm = loop {
                            if let Some(cm) = hub.online_now() {
                                break cm;
                            }
                            anyhow::ensure!(start.elapsed() < INFO_WAIT, "Not connected to Steam yet.");
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        };
                        estimate::estimate(&cm.conn, app.0, os.os(), &language).await
                    }
                    None => {
                        let conn = content::anonymous_connection().await?;
                        estimate::estimate(&conn, app.0, os.os(), &language).await
                    }
                }
            }
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Extra(Extra::InstallInfo { app, result })).await;
            wake();
        });
    }

    // ---- reducer arm -------------------------------------------------------

    pub(crate) fn apply_extra(&mut self, ev: Extra) {
        let current = self.generation.load(Ordering::Relaxed);
        match ev {
            Extra::UpdatesFound { generation, apps } if generation == current => {
                let now = downloads::now_unix();
                self.state.extra.update_check = UpdateCheck::Done { at: now, found: apps.len() };
                self.state.extra.updates = apps.iter().copied().collect();
                self.merge_installed();
                let hour = local_hour(now, super::local_utc_offset(now));
                for app in apps {
                    self.dl_auto_update(AppId(app), hour);
                }
            }
            Extra::UpdateCheckFailed { generation, error } if generation == current => {
                self.state.extra.update_check = UpdateCheck::Failed(error);
            }
            Extra::UpdateTick { generation } if generation == current => self.check_updates(),
            Extra::InstallInfo { app, result } => {
                let load = match result {
                    Ok(i) => Load::Ready(Arc::new(i)),
                    Err(e) => Load::Failed(e),
                };
                self.state.extra.install_info.insert(app.0, load);
            }
            Extra::MoveTotal { app, total } => {
                if let Some(m) = self.state.extra.moving.as_mut().filter(|m| m.app == app) {
                    m.total = total;
                }
            }
            Extra::MoveProgress { app, done } => {
                if let Some(m) = self.state.extra.moving.as_mut().filter(|m| m.app == app) {
                    m.done = done;
                }
            }
            Extra::MoveDone { app, result } => {
                self.extras.move_cancel = None;
                if let Some(m) = self.state.extra.moving.as_mut().filter(|m| m.app == app) {
                    if result.is_ok() {
                        m.done = m.total;
                    }
                    m.result = Some(result);
                }
                self.spawn_local_scan();
            }
            // Results of an older session.
            _ => {}
        }
    }
}
