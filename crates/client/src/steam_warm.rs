//! Steam before you press Play, and Kettle's own Steamworks host.
//!
//! Games that use the Steamworks API wait for the official client. So when Kettle starts and such a game is
//! installed (setting "Start Steam in the background", Automatic by default) the client is started quietly
//! (`-silent`, low CPU and disk priority, no window) while the library is still loading. Pressing Play then finds it
//! running and ready. The client stays running when Kettle quits, like Steam's own tray icon does.
//!
//! With "Run games without the Steam client (experimental)" on, nothing of the above happens: the games talk to
//! Kettle's own `steamclient.so`, served by [`crate::steamworks_host`] over a Unix socket.

use super::Client;
use crate::steamworks_host::{CmBackend, Server};
use sr_steam::local::InstalledApp;
use sr_steam::steam_client::{self as sc, Background, ScanCache, Started};
use sr_steam::store;
use sr_steamclient::proto::{AppInfo, socket_path};
use std::sync::Arc;

/// Kettle's side of the Steamworks socket, alive while own-steamclient mode is on and a session is open.
pub(crate) struct SwHost {
    _server: Server,
    pub(crate) backend: Arc<CmBackend>,
}

/// The cache stamp of an install: changes with a new build or an update.
pub(crate) fn stamp(app: &InstalledApp) -> u64 {
    app.build_id.wrapping_mul(1_000_003).wrapping_add(app.last_updated)
}

/// Do any of these installs use the Steamworks API? Answers come from the cache where they can; stops at the
/// first yes. Returns the answer and whether the cache changed.
pub(crate) fn any_uses(apps: &[InstalledApp], cache: &mut ScanCache) -> (bool, bool) {
    let mut changed = false;
    for a in apps {
        let (uses, c) = cache.uses(a.appid, &a.install_dir, stamp(a), sc::uses_steamworks);
        changed |= c;
        if uses {
            return (true, changed);
        }
    }
    (false, changed)
}

/// Installed games, tools excluded.
fn games_of(installed: &std::collections::HashMap<u32, InstalledApp>) -> Vec<InstalledApp> {
    let mut v: Vec<InstalledApp> = installed
        .values()
        .filter(|a| a.fully_installed() && !sr_steam::local::is_tool_app(a.appid, &a.name))
        .cloned()
        .collect();
    v.sort_by_key(|a| a.appid);
    v
}

impl Client {
    /// Start the official client quietly in the background when the setting asks for it. Runs on a worker thread.
    /// Called once the installed games are known and again when the setting changes; starts at most once per run
    /// of the decision (and never when the client already runs).
    pub(crate) fn warm_steam(&mut self) {
        if self.demo_active() {
            return;
        }
        let s = &self.state.settings;
        let mode = s.steam_background;
        if s.own_steamclient || mode == Background::Never || self.warm_done {
            return;
        }
        let apps = games_of(&self.installed);
        if apps.is_empty() && mode != Background::Always {
            return; // the scan of installed games has not finished (or there are none): it calls us again
        }
        self.warm_done = true;
        self.rt.spawn_blocking(move || {
            let installed = sc::start_spec().is_some();
            let running = sc::is_running();
            let needs = if mode == Background::Automatic && installed && !running {
                let mut cache = ScanCache::load();
                let (yes, changed) = any_uses(&apps, &mut cache);
                if changed {
                    cache.save();
                }
                yes
            } else {
                false
            };
            if sc::should_start_background(mode, needs, installed, running) {
                match sc::start_background() {
                    Ok(Started::StartedNow) => eprintln!("[steam] started the client in the background"),
                    Ok(_) => {}
                    Err(e) => eprintln!("[steam] background start failed: {e:#}"),
                }
            }
        });
    }

    /// The setting changed or the session did: run (or stop) Kettle's Steamworks host.
    pub(crate) fn sync_steamworks_host(&mut self) {
        self.warm_done = false;
        if !self.state.settings.own_steamclient {
            self.sw_host = None;
            return;
        }
        if self.sw_host.is_some() {
            return;
        }
        let Some(hub) = self.hub.clone() else { return };
        let backend = Arc::new(CmBackend::new(hub, self.rt.handle().clone(), store::data_dir().join("steamworks")));
        match Server::start(&socket_path(), backend.clone()) {
            Ok(server) => {
                if let Some(s) = &self.session {
                    backend.set_persona(&s.account.account_name);
                }
                self.sw_host = Some(SwHost { _server: server, backend });
                self.push_to_host();
            }
            Err(e) => self.state.notice = Some(format!("Couldn't start the Steamworks service: {e}")),
        }
    }

    /// Tell the host what the library and the installed games say.
    pub(crate) fn push_to_host(&self) {
        let Some(h) = &self.sw_host else { return };
        h.backend
            .set_owned(self.state.games.iter().filter(|g| !self.local_only.contains(&g.app_id.0)).map(|g| g.app_id.0));
        for a in self.installed.values() {
            let info = AppInfo {
                install_dir: Some(a.install_dir.to_string_lossy().into_owned()),
                build_id: a.build_id.min(u64::from(u32::MAX)) as u32,
            };
            h.backend.set_app_info(a.appid, info);
        }
    }

    fn demo_active(&self) -> bool {
        self.demo_installed || std::env::var_os("KETTLE_DEMO").is_some()
    }
}

/// The choices the play worker needs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SteamPlay {
    pub background: Background,
    pub own: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn app(id: u32, dir: &str, build: u64) -> InstalledApp {
        InstalledApp {
            appid: id,
            name: format!("g{id}"),
            install_dir: PathBuf::from(dir),
            size_on_disk: 0,
            state_flags: 4,
            build_id: build,
            last_updated: 5,
        }
    }

    #[test]
    fn stamps_change_with_the_build() {
        assert_ne!(stamp(&app(1, "/a", 10)), stamp(&app(1, "/a", 11)));
        assert_eq!(stamp(&app(1, "/a", 10)), stamp(&app(2, "/b", 10)));
    }

    #[test]
    fn any_uses_goes_through_the_cache_and_stops_at_the_first_yes() {
        let d = std::env::temp_dir().join(format!("kettle-warm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        std::fs::write(d.join("b/libsteam_api.so"), b"").unwrap();
        let apps = [app(1, d.join("a").to_str().unwrap(), 1), app(2, d.join("b").to_str().unwrap(), 1)];
        let mut cache = ScanCache::default();
        assert_eq!(any_uses(&apps, &mut cache), (true, true));
        assert_eq!(cache.apps.len(), 2);
        // Delete the library: the cached answer is still used for the same build (startup stays fast)...
        std::fs::remove_file(d.join("b/libsteam_api.so")).unwrap();
        assert_eq!(any_uses(&apps, &mut cache), (true, false));
        // ...and a new build is looked at again.
        let apps = [app(1, d.join("a").to_str().unwrap(), 1), app(2, d.join("b").to_str().unwrap(), 2)];
        assert_eq!(any_uses(&apps, &mut cache), (false, true));
    }
}
