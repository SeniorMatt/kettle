//! Steam Cloud saves: fetch newer saves before a game starts, send changed ones after it ended, and ask the user
//! when both sides changed. The rules and the transfers are in `sr_steam::cloud` / `sr_steam::cloud_sync`; this
//! file wires them into launching, settings and events.
//!
//! * The sync before a launch runs on the launch worker thread, before anything is started. A conflict stops the
//!   launch and shows a dialog (`State::cloud.conflict`); the answer comes back as `Command::CloudResolve`, which
//!   starts the launch again with the answer. A failed download never blocks the game, but then nothing is
//!   uploaded after it (the upload only happens for launches whose download step succeeded).
//! * Settings: a global switch (`Settings::cloud_sync`, on by default) and a per-game "off" list
//!   (`cloud_games.json`).
//! * Everything is described in the game's launch log. `KETTLE_CLOUD_DRYRUN=1` logs the plan and changes nothing.

use super::{Client, Event, State};
use serde::{Deserialize, Serialize};
use sr_model::AppId;
use sr_steam::appinfo::Kv;
use sr_steam::cloud::{self, Env, Mapping, Resolution, SyncState, Target, Ufs};
use sr_steam::cloud_sync::{self, CmBackend, Job, Post, Pre};
use sr_steam::run::{self, Prepared, Runner};
use sr_steam::store;
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

/// Games with Steam Cloud switched off (the global switch is `Settings::cloud_sync`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudGames {
    #[serde(default)]
    pub off: BTreeSet<String>,
}

impl CloudGames {
    pub fn path() -> PathBuf {
        store::data_dir().join("cloud_games.json")
    }

    pub fn load() -> CloudGames {
        store::load_json(&CloudGames::path()).unwrap_or_default()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        store::save_json(&CloudGames::path(), self)
    }

    pub fn is_off(&self, app: AppId) -> bool {
        self.off.contains(&app.0.to_string())
    }

    pub fn set_enabled(&mut self, app: AppId, on: bool) {
        if on {
            self.off.remove(&app.0.to_string());
        } else {
            self.off.insert(app.0.to_string());
        }
    }
}

/// What the sync of a game is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudPhase {
    /// Before the launch: reading the cloud and fetching newer saves.
    Downloading,
    /// After the game ended: sending changed saves.
    Uploading,
}

/// The user's answer to a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloudChoice {
    /// Send this computer's saves to the cloud, then start.
    KeepLocal,
    /// Replace this computer's saves with the cloud's (the old ones are copied to a backup folder), then start.
    KeepCloud,
    /// Don't start the game.
    Cancel,
}

/// Saves changed here and in the cloud since the last sync. Times are seconds since 1970 (0 = unknown).
#[derive(Debug, Clone, PartialEq)]
pub struct CloudConflict {
    pub app: AppId,
    /// The game's name.
    pub game: String,
    /// How many files differ.
    pub files: usize,
    /// The newest change among those files, on this computer and in the cloud.
    pub local_time: u64,
    pub cloud_time: u64,
}

/// What the UI may read about Steam Cloud.
#[derive(Debug, Clone, Default)]
pub struct CloudState {
    pub games: CloudGames,
    /// Games being synced right now.
    pub active: HashMap<AppId, CloudPhase>,
    /// A conflict waiting for an answer (the game is not started until then).
    pub conflict: Option<CloudConflict>,
    /// The last result per game, in words ("Downloaded 2 files", "Couldn't sync: ...").
    pub last: HashMap<AppId, String>,
}

impl State {
    /// Is Steam Cloud on for `app` (the global switch and the game's own)?
    pub fn cloud_enabled(&self, app: AppId) -> bool {
        self.settings.cloud_sync && !self.cloud.games.is_off(app)
    }
}

/// Where a game's saves are on this machine; kept from the sync before the launch for the one after it.
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    maps: Vec<Mapping>,
    remote: Option<PathBuf>,
    /// `remote/` was left out because it sits in the official Steam client's folder.
    remote_skipped: bool,
    target: Target,
}

#[derive(Debug)]
pub(crate) enum CloudEvent {
    Phase {
        app: AppId,
        phase: Option<CloudPhase>,
    },
    Conflict(CloudConflict),
    /// The sync before the launch worked: remember where the saves are, for the upload.
    Ready {
        app: AppId,
        ctx: Option<Ctx>,
    },
    /// A result in words. `problem` also goes to the status bar.
    Note {
        app: AppId,
        text: String,
        problem: bool,
    },
}

/// What a launch does about Steam Cloud.
pub(crate) enum CloudMode {
    Off,
    /// Switched on but not possible now (not connected); the reason goes to the log.
    Skip(String),
    On(CloudRun),
}

pub(crate) struct CloudRun {
    cm: std::sync::Arc<sr_steam::cm::Cm>,
    handle: tokio::runtime::Handle,
    steam_id: u64,
    resolution: Option<Resolution>,
    dry_run: bool,
}

/// What the launch worker does next.
pub(crate) enum Gate {
    /// Start the game; the text goes to the top of its launch log.
    Go(String),
    /// Don't start it (a conflict is waiting for the user).
    Stop,
}

fn dry_run() -> bool {
    std::env::var_os("KETTLE_CLOUD_DRYRUN").is_some_and(|v| v != "0")
}

fn lines(log: &[String]) -> String {
    log.iter().map(|l| format!("{l}\n")).collect()
}

fn plural(n: usize, what: &str) -> String {
    format!("{n} {what}{}", if n == 1 { "" } else { "s" })
}

/// The Wine prefix the game will run in (`.../pfx`), when it runs through Proton or umu.
fn prefix_of(plan: &run::LaunchPlan) -> Option<PathBuf> {
    plan.env
        .get("WINEPREFIX")
        .map(PathBuf::from)
        .or_else(|| plan.env.get("STEAM_COMPAT_DATA_PATH").map(|d| PathBuf::from(d).join("pfx")))
}

/// Where to look for saves for this launch, or why not.
fn locate(p: &Prepared, ufs: &Kv, steam_id: u64, appid: u32, log: &mut Vec<String>) -> Result<Ctx, String> {
    let prefix = match p.plan.runner {
        Runner::Native => None,
        Runner::Proton | Runner::Umu => {
            let pfx = prefix_of(&p.plan).ok_or("the game's Wine prefix is not known")?;
            if !pfx.join("system.reg").is_file() {
                // Writing into a prefix Proton has not set up yet would make it treat the prefix as damaged.
                return Err("the game's Wine prefix is not created yet; Steam Cloud starts with the next launch".into());
            }
            Some(pfx)
        }
        Runner::Direct => return Err("Steam Cloud is not supported for games started directly on Windows".into()),
    };
    let env = Env::for_launch(&p.install_dir, prefix.as_deref(), steam_id);
    let (maps, notes) = cloud::mappings(&Ufs::from_kv(ufs), &env);
    log.extend(notes.into_iter().map(|n| format!("cloud: {n}")));
    let official = run::steam_root_for(&p.install_dir);
    let remote = cloud::remote_dir(official.as_deref(), &store::data_dir(), env.account_id(), appid);
    if remote.is_none() {
        log.push(cloud::REMOTE_SKIPPED.to_string());
    }
    log.push(format!(
        "cloud: looking for saves as a {} game in {} place(s){}",
        env.target.name(),
        maps.len(),
        remote.as_ref().map_or(String::new(), |r| format!(" and {}", r.display()))
    ));
    let remote_skipped = remote.is_none();
    Ok(Ctx { maps, remote, remote_skipped, target: env.target })
}

fn job_for<'a>(appid: u32, ctx: &'a Ctx, dry: bool) -> Job<'a> {
    Job {
        appid,
        maps: &ctx.maps,
        remote: ctx.remote.as_deref(),
        target: ctx.target,
        dry_run: dry,
        state_path: SyncState::path(appid),
        backup_dir: store::data_dir().join("cloud-backups").join(appid.to_string()),
    }
}

/// On the launch worker, after the launch is prepared and before the game or the Steam client is started.
pub(crate) fn pre_launch(
    app: AppId,
    name: &str,
    p: &Prepared,
    ufs: Option<&Kv>,
    mode: &CloudMode,
    send: &impl Fn(Event),
) -> Gate {
    let ev = |e: CloudEvent| send(Event::Cloud(e));
    let run = match mode {
        CloudMode::Off => return Gate::Go(String::new()),
        CloudMode::Skip(why) => return Gate::Go(format!("cloud: not synced: {why}\n")),
        CloudMode::On(r) => r,
    };
    let done = |ctx: Option<Ctx>| {
        ev(CloudEvent::Ready { app, ctx });
        ev(CloudEvent::Phase { app, phase: None });
    };
    let Some(ufs) = ufs else {
        done(None);
        return Gate::Go("cloud: this game has no Steam Cloud data; nothing to sync\n".into());
    };
    let mut log = Vec::new();
    let ctx = match locate(p, ufs, run.steam_id, app.0, &mut log) {
        Ok(c) => c,
        Err(why) => {
            log.push(format!("cloud: not synced: {why}"));
            done(None);
            return Gate::Go(lines(&log));
        }
    };
    let mut be = CmBackend { conn: run.cm.conn.clone(), handle: run.handle.clone(), appid: app.0 };
    let dry = run.dry_run || dry_run();
    let result = cloud_sync::before_launch(&job_for(app.0, &ctx, dry), &mut be, run.resolution, &mut log);
    match result {
        Ok(Pre::Synced { downloaded, uploaded }) => {
            let text = match (downloaded, uploaded) {
                (0, 0) => "Steam Cloud: saves are up to date".to_string(),
                (d, 0) => format!("Steam Cloud: downloaded {}", plural(d, "save file")),
                (0, u) => format!("Steam Cloud: uploaded {}", plural(u, "save file")),
                (d, u) => format!("Steam Cloud: downloaded {}, uploaded {}", plural(d, "save file"), u),
            };
            let text = if ctx.remote_skipped { format!("{text}. {}", cloud::REMOTE_SKIPPED) } else { text };
            ev(CloudEvent::Note { app, text, problem: false });
            done(Some(ctx));
            Gate::Go(lines(&log))
        }
        Ok(Pre::Conflict { conflicts, local_time, cloud_time }) => {
            run::append_log(app.0, &lines(&log));
            ev(CloudEvent::Conflict(CloudConflict {
                app,
                game: name.to_string(),
                files: conflicts.len(),
                local_time,
                cloud_time,
            }));
            ev(CloudEvent::Phase { app, phase: None });
            Gate::Stop
        }
        Err(e) => {
            log.push(format!("cloud: sync failed: {e:#}"));
            // The game still starts, but nothing is uploaded after it: the cloud was not read successfully.
            ev(CloudEvent::Note {
                app,
                text: format!(
                    "Steam Cloud could not sync {name}: {e:#}. Your saves will not be uploaded after this session."
                ),
                problem: true,
            });
            done(None);
            Gate::Go(lines(&log))
        }
    }
}

impl Client {
    /// What a launch of `app` does about Steam Cloud (takes the user's conflict answer, if there is one).
    pub(crate) fn cloud_mode(&mut self, app: AppId) -> CloudMode {
        if !self.state.cloud_enabled(app) {
            return CloudMode::Off;
        }
        let Some(session) = &self.session else { return CloudMode::Off };
        let steam_id = session.account.steam_id.0;
        let resolution = self.cloud_choice.remove(&app);
        match self.hub.as_ref().and_then(|h| h.online_now()) {
            None => CloudMode::Skip("not connected to Steam".into()),
            Some(cm) => {
                self.state.cloud.active.insert(app, CloudPhase::Downloading);
                CloudMode::On(CloudRun { cm, handle: self.rt.handle().clone(), steam_id, resolution, dry_run: false })
            }
        }
    }

    pub(crate) fn set_cloud_sync(&mut self, on: bool) {
        self.state.settings.cloud_sync = on;
        self.save_settings();
    }

    pub(crate) fn set_game_cloud(&mut self, app: AppId, on: bool) {
        let mut games = self.state.cloud.games.clone();
        games.set_enabled(app, on);
        match games.save() {
            Ok(()) => self.state.cloud.games = games,
            Err(e) => self.state.notice = Some(format!("Couldn't save the Steam Cloud choice: {e:#}")),
        }
    }

    pub(crate) fn cloud_resolve(&mut self, app: AppId, choice: CloudChoice) {
        if self.state.cloud.conflict.as_ref().is_none_or(|c| c.app != app) {
            return;
        }
        self.state.cloud.conflict = None;
        match choice {
            CloudChoice::Cancel => {}
            CloudChoice::KeepLocal => {
                self.cloud_choice.insert(app, Resolution::KeepLocal);
                self.play(app);
            }
            CloudChoice::KeepCloud => {
                self.cloud_choice.insert(app, Resolution::KeepCloud);
                self.play(app);
            }
        }
    }

    /// A launch ended without a start (failure): forget what the sync before it left behind.
    pub(crate) fn cloud_launch_failed(&mut self, app: AppId) {
        self.state.cloud.active.remove(&app);
        self.cloud_ctx.remove(&app);
    }

    pub(crate) fn apply_cloud(&mut self, ev: CloudEvent) {
        match ev {
            CloudEvent::Phase { app, phase: Some(p) } => {
                self.state.cloud.active.insert(app, p);
            }
            CloudEvent::Phase { app, phase: None } => {
                self.state.cloud.active.remove(&app);
            }
            CloudEvent::Conflict(c) => self.state.cloud.conflict = Some(c),
            CloudEvent::Ready { app, ctx } => match ctx {
                Some(c) => {
                    self.cloud_ctx.insert(app, c);
                }
                None => {
                    self.cloud_ctx.remove(&app);
                }
            },
            CloudEvent::Note { app, text, problem } => {
                if problem {
                    self.state.notice = Some(text.clone());
                }
                self.state.cloud.last.insert(app, text);
            }
        }
    }

    /// After a game ended: send changed saves, when its download step worked and Cloud is still on.
    pub(crate) fn cloud_after_exit(&mut self, app: AppId) {
        let Some(ctx) = self.cloud_ctx.remove(&app) else { return };
        if !self.state.cloud_enabled(app) {
            return;
        }
        let Some(cm) = self.hub.as_ref().and_then(|h| h.online_now()) else {
            self.state
                .cloud
                .last
                .insert(app, "Steam Cloud: not connected, saves will be sent after the next session".into());
            run::append_log(
                app.0,
                "cloud: not connected to Steam after the game ended; saves are sent after the next session\n",
            );
            return;
        };
        self.state.cloud.active.insert(app, CloudPhase::Uploading);
        let (tx, wake, handle) = (self.tx.clone(), self.wake.clone(), self.rt.handle().clone());
        self.rt.spawn_blocking(move || {
            let send = |e: CloudEvent| {
                let _ = tx.blocking_send(Event::Cloud(e));
                wake();
            };
            let mut log = vec!["cloud: after the game ended".to_string()];
            let mut be = CmBackend { conn: cm.conn.clone(), handle, appid: app.0 };
            let (text, problem) = match cloud_sync::after_exit(&job_for(app.0, &ctx, dry_run()), &mut be, &mut log) {
                Ok(Post::Uploaded(n)) => (format!("Steam Cloud: uploaded {}", plural(n, "save file")), false),
                Ok(Post::NothingToDo) => ("Steam Cloud: saves are up to date".to_string(), false),
                Ok(Post::Conflict(n)) => (
                    format!("Steam Cloud: {} also changed in the cloud during play, so nothing was uploaded. Start the game again to choose.", plural(n, "save file")),
                    true,
                ),
                Err(e) => {
                    log.push(format!("cloud: upload failed: {e:#}"));
                    (format!("Steam Cloud could not upload your saves: {e:#}. They stay on this computer and are sent after the next session."), true)
                }
            };
            let text = if ctx.remote_skipped { format!("{text}. {}", cloud::REMOTE_SKIPPED) } else { text };
            run::append_log(app.0, &lines(&log));
            send(CloudEvent::Note { app, text, problem });
            send(CloudEvent::Phase { app, phase: None });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_game_switch_round_trips() {
        let mut g = CloudGames::default();
        assert!(!g.is_off(AppId(7)));
        g.set_enabled(AppId(7), false);
        assert!(g.is_off(AppId(7)) && !g.is_off(AppId(8)));
        let back: CloudGames = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert_eq!(back, g);
        g.set_enabled(AppId(7), true);
        assert!(g.off.is_empty());
        assert_eq!(serde_json::from_str::<CloudGames>("{}").unwrap(), CloudGames::default());
    }

    #[test]
    fn wording() {
        assert_eq!(plural(1, "save file"), "1 save file");
        assert_eq!(plural(3, "save file"), "3 save files");
        assert_eq!(lines(&["a".into(), "b".into()]), "a\nb\n");
    }
}
