//! What the lower half of a game's page needs: news, achievements and DLC, loaded when the page is opened
//! (`Command::LoadGamePage`) and kept per app. Everything is read-only; a section without data stays empty.

use crate::friends::{Friend, FriendsState};
use crate::{Client, Event, Load, demo};
use sr_model::AppId;
use sr_steam::content::estimate;
use sr_steam::gamepage::{self, Achievements, NewsItem};
use sr_steam::local;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Apps whose page data is kept; older ones are dropped.
const KEPT: usize = 24;
const DLC_WAIT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq)]
pub struct DlcEntry {
    pub appid: u32,
    pub name: String,
    /// The DLC has its own install folder on this machine.
    pub installed: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GamePageState {
    pub news: HashMap<u32, Load<Arc<Vec<NewsItem>>>>,
    /// `None` = no achievements (or no readable local files).
    pub achievements: HashMap<u32, Option<Arc<Achievements>>>,
    pub dlc: HashMap<u32, Load<Arc<Vec<DlcEntry>>>>,
    /// The last session with its screenshots, while its summary is open.
    pub post_game: HashMap<u32, crate::PostGame>,
    /// Games whose "Would you recommend" box is hidden right now.
    pub recommend_hidden: std::collections::HashSet<u32>,
    /// Demo mode: friends and the numbers come from `demo`.
    pub demo: bool,
}

impl GamePageState {
    /// Friends in this game right now (in demo mode two made-up ones).
    pub fn friends_playing(&self, friends: &FriendsState, app: AppId, name: &str) -> Vec<Friend> {
        if self.demo {
            return demo::playing(app.0, name);
        }
        friends
            .friends
            .iter()
            .filter(|f| f.game.as_ref().is_some_and(|g| g.app_id == app.0) && f.state.is_online())
            .cloned()
            .collect()
    }

    pub fn news(&self, app: AppId) -> Option<&Arc<Vec<NewsItem>>> {
        self.news.get(&app.0).and_then(Load::ready).filter(|n| !n.is_empty())
    }

    pub fn achievements(&self, app: AppId) -> Option<&Arc<Achievements>> {
        self.achievements.get(&app.0).and_then(Option::as_ref)
    }

    pub fn dlc(&self, app: AppId) -> Option<&Arc<Vec<DlcEntry>>> {
        self.dlc.get(&app.0).and_then(Load::ready).filter(|d| !d.is_empty())
    }
}

#[derive(Debug)]
pub(crate) enum GamePageEv {
    News { app: u32, result: Result<Vec<NewsItem>, String> },
    Achievements { app: u32, data: Option<Achievements> },
    Dlc { app: u32, result: Result<Vec<(u32, String)>, String> },
}

/// Page data is refreshed when it is older than this (the news cache on disk has the same age).
const MAX_AGE: Duration = gamepage::NEWS_TTL;

impl Client {
    pub(crate) fn load_game_page(&mut self, app: AppId) {
        self.refresh_post_game(app);
        if self.state.gamepage.demo {
            self.state.gamepage.news.insert(app.0, Load::Ready(Arc::new(demo::news(app.0))));
            self.state.gamepage.achievements.insert(app.0, Some(Arc::new(demo::achievements(app.0))));
            let dlc = demo::dlc(app.0)
                .into_iter()
                .map(|(appid, name, installed)| DlcEntry { appid, name, installed })
                .collect();
            self.state.gamepage.dlc.insert(app.0, Load::Ready(Arc::new(dlc)));
            return;
        }
        if self.gamepage_at.get(&app.0).is_some_and(|t| t.elapsed() < MAX_AGE) {
            return;
        }
        if self.gamepage_at.len() >= KEPT {
            let old = self.gamepage_at.iter().min_by_key(|(_, t)| **t).map(|(k, _)| *k);
            if let Some(k) = old {
                self.gamepage_at.remove(&k);
                self.state.gamepage.news.remove(&k);
                self.state.gamepage.achievements.remove(&k);
                self.state.gamepage.dlc.remove(&k);
            }
        }
        self.gamepage_at.insert(app.0, Instant::now());
        let gp = &mut self.state.gamepage;
        if !matches!(gp.news.get(&app.0), Some(Load::Ready(_))) {
            gp.news.insert(app.0, Load::Loading);
        }
        let (tx, wake) = (self.tx.clone(), self.wake.clone());

        // News: one public request, cached for an hour on disk.
        {
            let (tx, wake) = (tx.clone(), wake.clone());
            let id = app.0;
            self.rt.spawn_blocking(move || {
                let result = gamepage::fetch_news(id).map_err(|e| format!("{e:#}"));
                let _ = tx.blocking_send(Event::GamePage(GamePageEv::News { app: id, result }));
                wake();
            });
        }
        // Achievements: the official client's stats files (read only) and our own stats file.
        {
            let (tx, wake) = (tx.clone(), wake.clone());
            let id = app.0;
            let account = self.state.account.as_ref().map_or(0, |a| (a.steam_id.0 & 0xffff_ffff) as u32);
            let steam_id = self.state.account.as_ref().map_or(0, |a| a.steam_id.0);
            self.rt.spawn_blocking(move || {
                let own = own_unlocked(steam_id, id);
                let data = local::steam_roots().iter().find_map(|r| gamepage::read_local(r, id, account, &own));
                let _ = tx.blocking_send(Event::GamePage(GamePageEv::Achievements { app: id, data }));
                wake();
            });
        }
        // DLC names over the signed-in connection (nothing is asked while signed out).
        if let (Some(hub), true) = (self.hub.clone(), self.session.is_some()) {
            let id = app.0;
            self.state.gamepage.dlc.entry(id).or_insert(Load::Loading);
            self.rt.spawn(async move {
                let result = async {
                    let start = Instant::now();
                    let cm = loop {
                        if let Some(cm) = hub.online_now() {
                            break cm;
                        }
                        anyhow::ensure!(start.elapsed() < DLC_WAIT, "Not connected to Steam yet.");
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    };
                    let info = estimate::estimate(&cm.conn, id, None, "english").await?;
                    Ok(info.dlc.into_iter().map(|d| (d.appid, d.name)).collect::<Vec<_>>())
                }
                .await
                .map_err(|e: anyhow::Error| format!("{e:#}"));
                let _ = tx.send(Event::GamePage(GamePageEv::Dlc { app: id, result })).await;
                wake();
            });
        }
    }

    pub(crate) fn apply_game_page(&mut self, ev: GamePageEv) {
        match ev {
            GamePageEv::News { app, result } => {
                let load = match result {
                    Ok(v) => Load::Ready(Arc::new(v)),
                    Err(e) => Load::Failed(e),
                };
                self.state.gamepage.news.insert(app, load);
            }
            GamePageEv::Achievements { app, data } => {
                self.state.gamepage.achievements.insert(app, data.map(Arc::new));
            }
            GamePageEv::Dlc { app, result } => {
                let load = match result {
                    Ok(v) => Load::Ready(Arc::new(
                        v.into_iter()
                            .filter(|(_, n)| !n.trim().is_empty())
                            .map(|(appid, name)| DlcEntry {
                                appid,
                                name,
                                installed: self.installed.contains_key(&appid),
                            })
                            .collect(),
                    )),
                    Err(e) => Load::Failed(e),
                };
                self.state.gamepage.dlc.insert(app, load);
            }
        }
    }
}

/// Achievements Kettle's own Steamworks core recorded (`<data>/stats/<steam id>/<app>.json`), if that file exists.
fn own_unlocked(steam_id: u64, app: u32) -> Vec<(String, u64)> {
    let path = sr_steam::store::data_dir().join("stats").join(steam_id.to_string()).join(format!("{app}.json"));
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return Vec::new() };
    v["achievements"]
        .as_object()
        .map(|m| m.iter().map(|(k, t)| (k.clone(), t.as_u64().unwrap_or(0))).collect())
        .unwrap_or_default()
}
