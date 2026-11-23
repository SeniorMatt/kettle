//! Starting games with our own launcher (`sr_steam::run`), tracking which ones run, and local play time.
//!
//! Preparing a launch reads files (appinfo.vdf, tool folders), so it happens on a worker thread; the UI only
//! sees events. Play time is recorded here, in a small JSON file, because we don't report to Steam: it is
//! added to the number Steam gave us when the library is shown.

use super::cloud::{self, CloudMode, Gate};
use super::{Client, Event, Waker};
use serde::{Deserialize, Serialize};
use sr_model::AppId;
use sr_steam::local::InstalledApp;
use sr_steam::{run, store};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use tokio::sync::mpsc;

/// A game we started and that has not exited yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningGame {
    pub pid: u32,
    /// When it started (for the session time in the overlay).
    pub started: std::time::Instant,
    /// Started inside Gamescope: the overlay helper joins Gamescope's X server.
    pub gamescope: bool,
}

/// Why a game could not be started, in words for the user.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayProblem {
    pub app: AppId,
    pub name: String,
    pub message: String,
    /// A missing or unusable compatibility tool: the UI offers the Proton manager.
    pub needs_compat_tool: bool,
    /// What a bug report needs: the command, the variables we set and the end of the game's log (home folder
    /// shown as `~`, secrets hidden). Empty when nothing was started. The UI's "Copy details" copies this.
    pub details: String,
    /// The launch log (`<data dir>/logs/launch-<appid>.log`), when one was written.
    pub log: Option<PathBuf>,
}

/// Play time we measured ourselves.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PlayStats {
    /// `appid -> (seconds played, unix time of the last run)`
    pub apps: BTreeMap<String, (u64, u64)>,
}

impl PlayStats {
    pub fn path() -> PathBuf {
        store::data_dir().join("playtime.json")
    }

    pub fn load() -> PlayStats {
        store::load_json(&PlayStats::path()).unwrap_or_default()
    }

    pub fn add(&mut self, app: AppId, seconds: u64, now: u64) {
        let e = self.apps.entry(app.0.to_string()).or_default();
        e.0 += seconds;
        e.1 = now;
    }

    pub fn get(&self, app: AppId) -> (u64, u64) {
        self.apps.get(&app.0.to_string()).copied().unwrap_or_default()
    }
}

/// How long we wait for the Steam client to come up.
const STEAM_START_TIMEOUT: Duration = Duration::from_secs(90);
/// A game that needs the client but exits within this time gets an explanation.
const EARLY_EXIT_SECS: u64 = 10;

/// Does this error mean "no usable compatibility tool"?
fn is_compat_problem(message: &str) -> bool {
    message.contains("compatibility tool") || message.contains("is a runtime")
}

impl Client {
    pub(crate) fn play(&mut self, app: AppId) {
        // An update is waiting: the UI asks "Update now / Play anyway" first.
        if self.update_gate(app) {
            return;
        }
        if self.state.running.contains_key(&app)
            || self.state.starting_steam.contains(&app)
            || self.state.cloud.active.contains_key(&app)
            || self.state.cloud.conflict.as_ref().is_some_and(|c| c.app == app)
        {
            return;
        }
        if let Some(g) = self.state.non_steam.get(app).cloned() {
            return self.play_non_steam(g);
        }
        let name = self.state.game(app).map(|g| g.name.clone()).unwrap_or_default();
        let installed = self.installed.get(&app.0).cloned();
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let steam = super::steam_warm::SteamPlay {
            background: self.state.settings.steam_background,
            own: self.state.settings.own_steamclient,
        };
        let stamp = installed.as_ref().map_or(0, super::steam_warm::stamp);
        let (ours, account) = self.launch_options_for(app);
        let options = LaunchText {
            ours,
            account,
            hud: self.state.settings.hud.clone(),
            proton_log: self.state.settings.proton_log,
            gamescope: self.gamescope_for(app).then(|| self.state.settings.gamescope.clone()),
            cloud: self.cloud_mode(app),
            env: self.state.game_env.get(app),
        };

        self.rt.spawn_blocking(move || launch(app, name, installed, (steam, stamp), options, tx, wake));
    }

    /// Stop a running game: end its whole process group (the game, Proton and the helpers).
    pub(crate) fn stop_game(&mut self, app: AppId) {
        let Some(g) = self.state.running.get(&app) else { return };
        let pid = g.pid;
        if pid == 0 {
            return; // a simulated game (KETTLE_DEMO_RUNNING)
        }
        self.rt.spawn_blocking(move || terminate_group(pid));
    }

    pub(crate) fn game_started(&mut self, app: AppId, pid: u32, gamescope: bool) {
        self.state.starting_steam.retain(|a| *a != app);
        self.state.running.insert(app, RunningGame { pid, started: std::time::Instant::now(), gamescope });
        self.apply_bandwidth();
    }

    pub(crate) fn game_exited(&mut self, app: AppId, seconds: u64, success: bool) {
        self.state.running.remove(&app);
        self.apply_bandwidth();
        self.cloud_after_exit(app);
        self.play_stats.add(app, seconds, super::downloads::now_unix());
        let _ = store::save_json(&PlayStats::path(), &self.play_stats);
        self.merge_installed();
        let missing = self.steam_missing.remove(&app);
        if let Some(message) = missing.filter(|_| seconds < EARLY_EXIT_SECS) {
            self.state.notice = Some(message);
        } else if !success && seconds < 5 {
            let name = self.state.game(app).map(|g| g.name.clone()).unwrap_or_else(|| format!("App {}", app.0));
            self.state.play_problem = Some(early_exit_problem(app, name));
        }
    }

    pub(crate) fn play_failed(&mut self, problem: PlayProblem) {
        self.state.starting_steam.retain(|a| *a != problem.app);
        self.steam_missing.remove(&problem.app);
        self.cloud_launch_failed(problem.app);
        self.state.play_problem = Some(problem);
    }

    pub(crate) fn steam_starting(&mut self, app: AppId) {
        if !self.state.starting_steam.contains(&app) {
            self.state.starting_steam.push(app);
        }
    }

    pub(crate) fn steam_missing(&mut self, app: AppId, message: String) {
        self.state.starting_steam.retain(|a| *a != app);
        self.steam_missing.insert(app, message);
    }
}

/// Where a launch finds the game's launch options: our saved text, else the official client's for `account`.
struct LaunchText {
    ours: Option<String>,
    account: Option<u32>,
    /// The performance overlay setting at the time of the launch.
    hud: sr_steam::mangohud::HudSettings,
    /// "Write Proton logs" (Settings): `PROTON_LOG=1` for Windows games.
    proton_log: bool,
    /// Gamescope's settings when this game is to run in it (the global switch and the game's override resolved).
    gamescope: Option<sr_steam::gamescope::GamescopeSettings>,
    /// What this launch does about Steam Cloud.
    cloud: CloudMode,
    /// The game's own environment variables and switches (see `game_env`).
    env: crate::GameEnv,
}

/// Worker: prepare the plan from Steam's app data and start it.
fn launch(
    app: AppId,
    name: String,
    installed: Option<InstalledApp>,
    (steam, stamp): (super::steam_warm::SteamPlay, u64),
    options: LaunchText,
    tx: mpsc::Sender<Event>,
    wake: Waker,
) {
    let info = run::appinfo_for(app.0);
    // Steam Cloud's save rules come with the app data.
    let ufs = info.as_ref().ok().and_then(|i| i.as_ref()).and_then(|i| i.ufs.clone());
    let prepared = info.and_then(|info| {
        let info = info.ok_or_else(|| {
            anyhow::anyhow!(
                "Steam's app data has no launch information for this game yet. Try \"Launch via Steam client\"."
            )
        })?;
        // Without an account (demo, signed out) the official client's value is not looked up.
        let launch_options = match options.account {
            Some(_) => sr_steam::launch_opts::effective(app.0, options.ours.as_deref(), options.account),
            None => options.ours.clone(),
        };
        // Gamescope: only when asked for and installed; otherwise the game starts as usual.
        let det = sr_steam::gamescope::detect();
        let gamescope = options.gamescope.as_ref().and_then(|g| {
            let l = sr_steam::gamescope::plan_launch(true, g, &options.hud, app.0, &det, false);
            if l.is_none() {
                eprintln!("[gamescope] not installed; starting the game without it");
            }
            l
        });
        // The frame cap goes to Gamescope then, so the HUD must not cap a second time.
        let hud_settings = sr_steam::gamescope::hud_for(&options.hud, gamescope.is_some());
        // The old display report would make the overlay join the last session's display.
        sr_steam::gamescope::clear_display(app.0);
        // MangoHud: writes the game's config file; a problem here only means no HUD, never a failed launch.
        let hud = sr_steam::mangohud::prepare(&hud_settings, app.0).unwrap_or_else(|e| {
            eprintln!("[hud] not applied: {e:#}");
            None
        });
        run::prepare_with(
            &info,
            installed.as_ref(),
            &run::PrepareOpts { launch_options, hud, proton_log: options.proton_log, gamescope, ..Default::default() },
        )
        .map(|mut p| {
            // The game's own variables go last, so they win over everything we set.
            options.env.apply(&mut p.plan, super::game_env::find_gamemoderun());
            p
        })
    });
    let send = move |ev: Event| {
        let _ = tx.blocking_send(ev);
        wake();
    };
    // Steam Cloud: fetch newer saves first. A conflict stops the launch here; the dialog starts it again.
    let mut preamble = String::new();
    let prepared = match prepared {
        Ok(p) => match cloud::pre_launch(app, &name, &p, ufs.as_ref(), &options.cloud, &send) {
            Gate::Go(text) => {
                preamble = text;
                Ok(p)
            }
            Gate::Stop => return,
        },
        Err(e) => Err(e),
    };
    let prepared = prepared.map(|mut p| {
        prepare_steam(app, &mut p.plan, &p.install_dir, stamp, steam, &send);
        p
    });
    let (result, details) = match prepared {
        Ok(p) => (start(&p.plan, &preamble, &send).map(|pid| (pid, p.gamescope)), run::describe(&p.plan)),
        Err(e) => (Err(e), String::new()),
    };
    match result {
        Ok((pid, gamescope)) => send(Event::GameStarted { app, pid, gamescope }),
        Err(e) => {
            let message = format!("{e:#}");
            let needs_compat_tool = is_compat_problem(&message);
            let details = format!("{name} ({})\n{details}\nerror: {message}\n", app.0);
            let log = Some(run::log_path(app.0)).filter(|p| p.is_file());
            send(Event::PlayFailed(PlayProblem { app, name, message, needs_compat_tool, details, log }));
        }
    }
}

/// A game that ended within seconds with an error: say why when the log tells, and carry the log's end.
fn early_exit_problem(app: AppId, name: String) -> PlayProblem {
    let tail = run::log_tail(app.0, 60);
    let log = run::log_path(app.0);
    let reason = explain_log(&tail).unwrap_or("The game ended right away.");
    PlayProblem {
        app,
        message: format!("{reason}\nThe full log is {}", log.display()),
        needs_compat_tool: false,
        details: format!("{name} ({})\n{tail}\n", app.0),
        log: Some(log).filter(|p| p.is_file()),
        name,
    }
}

/// A plain-words reason for the failures the logs show most often.
fn explain_log(tail: &str) -> Option<&'static str> {
    let has = |s: &str| tail.contains(s);
    if (has("C:\\windows") && (has("failed") || has("not found"))) || has("C:\\\\windows") {
        Some(
            "Proton could not set up the Windows environment (the \"C:\\windows ... failed\" error). The game's prefix \
             may be damaged or the compatibility tool cannot run here; try another tool for this game.",
        )
    } else if has("bwrap:") || (has("pressure-vessel") && has("error")) {
        Some("The Steam Linux Runtime container could not start (see the log for the bwrap / pressure-vessel line).")
    } else if has("No such file or directory") {
        Some("A file the game or Proton needs is missing (\"No such file or directory\" in the log).")
    } else {
        None
    }
}

/// Steam for one launch. Games without the Steamworks API never wait for anything. For the others: in own mode the
/// game gets Kettle's steamclient (and Steam is never started); otherwise the official client must be up, which
/// costs nothing when the background start already did it.
fn prepare_steam(
    app: AppId,
    plan: &mut run::LaunchPlan,
    install_dir: &std::path::Path,
    stamp: u64,
    steam: super::steam_warm::SteamPlay,
    send: &impl Fn(Event),
) {
    use sr_steam::steam_client::{Background, ScanCache};
    if steam.background == Background::Never && !steam.own {
        return;
    }
    let mut cache = ScanCache::load();
    let (uses, changed) = cache.uses(app.0, install_dir, stamp, sr_steam::steam_client::uses_steamworks);
    if changed {
        cache.save();
    }
    if !uses {
        return;
    }
    if steam.own {
        match use_own_steamclient(app, plan) {
            Ok(()) => return,
            Err(why) => {
                eprintln!("[steamclient] own mode not used: {why}");
                if steam.background == Background::Never {
                    return;
                }
                // Fall through to the official client; say why if the game then fails early.
                send(Event::SteamMissing { app, message: format!("{why}. Kettle started the Steam client instead.") });
            }
        }
    }
    ensure_steam(app, send);
}

/// Point a game at Kettle's own steamclient: the library is installed for the game, the socket and the app id are
/// in its environment. Any problem is an `Err` with a sentence, and the caller falls back.
fn use_own_steamclient(app: AppId, plan: &mut run::LaunchPlan) -> Result<(), String> {
    let src = super::steamroot::library_source()
        .ok_or("Kettle's own steamclient library (libsteamclient.so) was not found next to Kettle")?;
    let home = std::env::var_os("HOME").map(PathBuf::from).ok_or("no home folder")?;
    let root = super::steamroot::prepare(&super::steamroot::default_root(), &src, &home)?;
    own_env(&mut plan.env, app, &root, &sr_steamclient::proto::socket_path());
    Ok(())
}

/// The variables a game needs to use Kettle's steamclient.
fn own_env(env: &mut BTreeMap<String, String>, app: AppId, root: &std::path::Path, sock: &std::path::Path) {
    let a = app.0.to_string();
    env.insert("SteamAppId".into(), a.clone());
    env.insert("SteamGameId".into(), a);
    env.insert("KETTLE_STEAMCLIENT_SOCK".into(), sock.to_string_lossy().into_owned());
    env.insert("STEAM_COMPAT_CLIENT_INSTALL_PATH".into(), root.to_string_lossy().into_owned());
}

/// The game needs the official client: start it if it is not running and wait until its IPC answers (no fixed
/// delay; no waiting at all when it is up). Never fails the launch; if the client can't be started the game still
/// starts, with a note for an early exit.
fn ensure_steam(app: AppId, send: &impl Fn(Event)) {
    use sr_steam::steam_client::{self as sc, Started};
    if sc::is_running() && sc::ipc_ready() {
        return;
    }
    send(Event::SteamStarting { app });
    let why = match sc::ensure_running(STEAM_START_TIMEOUT, &AtomicBool::new(false)) {
        Ok(Started::AlreadyRunning | Started::StartedNow) => return,
        Ok(Started::NotInstalled) => {
            "This game needs the Steam client, and none is installed. Install Steam, or the game can't start."
                .to_string()
        }
        Err(e) => format!("This game needs the Steam client, which could not be started ({e:#})."),
    };
    send(Event::SteamMissing { app, message: why });
}

/// Spawn `plan`; the exit watcher reports through `send`. Split out so a test can run a fake plan.
pub(crate) fn start(
    plan: &run::LaunchPlan,
    preamble: &str,
    send: &(impl Fn(Event) + Clone + Send + 'static),
) -> anyhow::Result<u32> {
    let app = AppId(plan.appid);
    let on_exit = send.clone();
    run::spawn_with_preamble(plan, preamble, move |f| {
        on_exit(Event::GameExited { app, seconds: f.played.as_secs(), success: f.success });
    })
}

#[cfg(unix)]
fn terminate_group(pid: u32) {
    // The game was started as its own process group leader, so `-pid` addresses the whole group.
    let _ = std::process::Command::new("kill").args(["-TERM", "--", &format!("-{pid}")]).status();
}

#[cfg(not(unix))]
fn terminate_group(pid: u32) {
    let _ = std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T"]).status();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex};

    /// A fake game: `/bin/true` as a plan. Checks the start / exit events and the log file.
    #[cfg(unix)]
    #[test]
    fn fake_plan_runs_and_reports_exit() {
        let app = 4_000_000_001;
        let plan = run::LaunchPlan {
            appid: app,
            runner: run::Runner::Native,
            program: PathBuf::from("/bin/true"),
            args: vec![],
            cwd: PathBuf::from("/"),
            env: BTreeMap::new(),
            create_dirs: vec![],
        };
        let (done_tx, done_rx) = channel();
        let done_tx = Arc::new(Mutex::new(done_tx));
        let send = move |ev: Event| {
            if let Event::GameExited { app, success, .. } = ev {
                let _ = done_tx.lock().unwrap().send((app, success));
            }
        };
        let pid = start(&plan, "", &send).expect("spawns");
        assert!(pid > 0);
        let (got, success) = done_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("exit event");
        assert_eq!((got, success), (AppId(app), true));
        let _ = std::fs::remove_file(run::log_path(app));
    }

    #[test]
    fn own_mode_sets_the_game_up() {
        let mut env = BTreeMap::new();
        own_env(&mut env, AppId(480), std::path::Path::new("/root"), std::path::Path::new("/run/s.sock"));
        assert_eq!(env["SteamAppId"], "480");
        assert_eq!(env["SteamGameId"], "480");
        assert_eq!(env["KETTLE_STEAMCLIENT_SOCK"], "/run/s.sock");
        assert_eq!(env["STEAM_COMPAT_CLIENT_INSTALL_PATH"], "/root");
    }

    #[test]
    fn stats_add_up() {
        let mut s = PlayStats::default();
        s.add(AppId(7), 90, 1000);
        s.add(AppId(7), 30, 2000);
        assert_eq!(s.get(AppId(7)), (120, 2000));
        assert_eq!(s.get(AppId(8)), (0, 0));
    }

    #[test]
    fn log_reasons() {
        assert!(
            explain_log("wine: could not load C:\\windows\\system32\\kernel32.dll, status c0000135 failed").is_some()
        );
        assert!(explain_log("bwrap: Can't create file").is_some());
        assert!(explain_log("fine").is_none());
    }

    #[test]
    fn compat_problems_are_recognised() {
        assert!(is_compat_problem("this game needs a compatibility tool (Proton), but none is installed or chosen"));
        assert!(!is_compat_problem("launch entry has an unsafe executable"));
    }
}
