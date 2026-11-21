//! Starting games with our own launcher (`sr_steam::run`), tracking which ones run, and local play time.
//!
//! Preparing a launch reads files (appinfo.vdf, tool folders), so it happens on a worker thread; the UI only
//! sees events. Play time is recorded here, in a small JSON file, because we don't report to Steam: it is
//! added to the number Steam gave us when the library is shown.

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
}

/// Why a game could not be started, in words for the user.
#[derive(Debug, Clone, PartialEq)]
pub struct PlayProblem {
    pub app: AppId,
    pub name: String,
    pub message: String,
    /// A missing or unusable compatibility tool: the UI offers the Proton manager.
    pub needs_compat_tool: bool,
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
        if self.state.running.contains_key(&app) || self.state.starting_steam.contains(&app) {
            return;
        }
        let name = self.state.game(app).map(|g| g.name.clone()).unwrap_or_default();
        let installed = self.installed.get(&app.0).cloned();
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        let start_steam = self.state.settings.start_steam;
        let (ours, account) = self.launch_options_for(app);
        let options = LaunchText { ours, account };
        self.rt.spawn_blocking(move || launch(app, name, installed, start_steam, options, tx, wake));
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

    pub(crate) fn game_started(&mut self, app: AppId, pid: u32) {
        self.state.starting_steam.retain(|a| *a != app);
        self.state.running.insert(app, RunningGame { pid, started: std::time::Instant::now() });
    }

    pub(crate) fn game_exited(&mut self, app: AppId, seconds: u64, success: bool) {
        self.state.running.remove(&app);
        self.play_stats.add(app, seconds, super::downloads::now_unix());
        let _ = store::save_json(&PlayStats::path(), &self.play_stats);
        self.merge_installed();
        let missing = self.steam_missing.remove(&app);
        if let Some(message) = missing.filter(|_| seconds < EARLY_EXIT_SECS) {
            self.state.notice = Some(message);
        } else if !success && seconds < 5 {
            let log = run::log_path(app.0);
            self.state.notice = Some(format!("The game ended right away. See {}", log.display()));
        }
    }

    pub(crate) fn play_failed(&mut self, problem: PlayProblem) {
        self.state.starting_steam.retain(|a| *a != problem.app);
        self.steam_missing.remove(&problem.app);
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
}

/// Worker: prepare the plan from Steam's app data and start it.
fn launch(
    app: AppId,
    name: String,
    installed: Option<InstalledApp>,
    start_steam: bool,
    options: LaunchText,
    tx: mpsc::Sender<Event>,
    wake: Waker,
) {
    let prepared = run::appinfo_for(app.0).and_then(|info| {
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
        run::prepare_with(&info, installed.as_ref(), &run::PrepareOpts { launch_options, ..Default::default() })
    });
    let send = move |ev: Event| {
        let _ = tx.blocking_send(ev);
        wake();
    };
    let prepared = prepared.inspect(|p| {
        if start_steam {
            ensure_steam(app, &p.install_dir, &send);
        }
    });
    match prepared.and_then(|p| start(&p.plan, &send)) {
        Ok(pid) => send(Event::GameStarted { app, pid }),
        Err(e) => {
            let message = format!("{e:#}");
            let needs_compat_tool = is_compat_problem(&message);
            send(Event::PlayFailed(PlayProblem { app, name, message, needs_compat_tool }));
        }
    }
}

/// Games that use the Steamworks API only start while the official client runs: start it first when allowed.
/// Never fails the launch; if the client can't be started the game still starts, with a note for an early exit.
fn ensure_steam(app: AppId, install_dir: &std::path::Path, send: &impl Fn(Event)) {
    use sr_steam::steam_client::{self as sc, Started};
    if !sc::uses_steamworks(install_dir) || sc::is_running() {
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
fn start(plan: &run::LaunchPlan, send: &(impl Fn(Event) + Clone + Send + 'static)) -> anyhow::Result<u32> {
    let app = AppId(plan.appid);
    let on_exit = send.clone();
    run::spawn(plan, move |f| {
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
        let pid = start(&plan, &send).expect("spawns");
        assert!(pid > 0);
        let (got, success) = done_rx.recv_timeout(std::time::Duration::from_secs(10)).expect("exit event");
        assert_eq!((got, success), (AppId(app), true));
        let _ = std::fs::remove_file(run::log_path(app));
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
    fn compat_problems_are_recognised() {
        assert!(is_compat_problem("this game needs a compatibility tool (Proton), but none is installed or chosen"));
        assert!(!is_compat_problem("launch entry has an unsafe executable"));
    }
}
