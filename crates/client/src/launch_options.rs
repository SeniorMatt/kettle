//! Per-game launch options: our saved values (`launch_options.json`) and, as the default, what the official
//! client has saved for the game (read from its `localconfig.vdf`, never written).

use super::{Client, Event};
use sr_model::AppId;
use sr_steam::launch_opts::{self, LaunchOptions};
use std::collections::{BTreeMap, HashMap};

/// What the UI may read.
#[derive(Debug, Clone, Default)]
pub struct LaunchOptionsState {
    /// Our values by app id; an empty text means "none".
    pub ours: BTreeMap<String, String>,
    /// The official client's value by app id, once asked for (`None` inside = it has none).
    pub steam: HashMap<u32, Option<String>>,
}

impl LaunchOptionsState {
    /// The text that applies: ours, else the official client's (once known).
    pub fn effective(&self, app: AppId) -> String {
        match self.ours.get(&app.0.to_string()) {
            Some(t) => t.clone(),
            None => self.steam.get(&app.0).cloned().flatten().unwrap_or_default(),
        }
    }

    /// Has the official client's value been looked up yet?
    pub fn steam_known(&self, app: AppId) -> bool {
        self.steam.contains_key(&app.0)
    }

    /// The official client's value, if it has one.
    pub fn steam_value(&self, app: AppId) -> Option<&str> {
        self.steam.get(&app.0).and_then(|v| v.as_deref())
    }
}

impl Client {
    /// The account folder id of the signed-in account (`None` in demo mode or when signed out).
    fn account_folder(&self) -> Option<u32> {
        self.state.account.as_ref().and_then(|a| launch_opts::account_id(a.steam_id.0))
    }

    /// Look up the official client's value for the game (a small file read, off the UI thread).
    pub(crate) fn load_launch_options(&mut self, app: AppId) {
        let Some(account) = self.account_folder() else {
            self.state.launch_options.steam.insert(app.0, None);
            return;
        };
        let (tx, wake) = (self.tx.clone(), self.wake.clone());
        self.rt.spawn_blocking(move || {
            let text = launch_opts::steam_default(app.0, Some(account));
            let _ = tx.blocking_send(Event::SteamLaunchOptions { app, text });
            wake();
        });
    }

    pub(crate) fn steam_launch_options(&mut self, app: AppId, text: Option<String>) {
        self.state.launch_options.steam.insert(app.0, text);
    }

    pub(crate) fn set_launch_options(&mut self, app: AppId, text: String) {
        let default = self.state.launch_options.steam_value(app).map(str::to_string);
        let mut saved = LaunchOptions { apps: self.state.launch_options.ours.clone() };
        saved.set(app.0, Some(&text), default.as_deref());
        match saved.save() {
            Ok(()) => self.state.launch_options.ours = saved.apps,
            Err(e) => self.state.notice = Some(format!("Couldn't save the launch options: {e:#}")),
        }
    }

    /// Our value for a launch, and the account folder for looking up the official client's.
    pub(crate) fn launch_options_for(&self, app: AppId) -> (Option<String>, Option<u32>) {
        (self.state.launch_options.ours.get(&app.0.to_string()).cloned(), self.account_folder())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_prefers_ours() {
        let mut s = LaunchOptionsState::default();
        let app = AppId(5);
        assert_eq!(s.effective(app), "");
        s.steam.insert(5, Some("-steam".into()));
        assert_eq!(s.effective(app), "-steam");
        s.ours.insert("5".into(), String::new());
        assert_eq!(s.effective(app), "");
        s.ours.insert("5".into(), "-mine".into());
        assert_eq!(s.effective(app), "-mine");
    }
}
