//! Kettle's side of the in-game overlay.
//!
//! The overlay is drawn by `libkettle_overlay.so` inside the game's own frames (crate `sr-overlay-hook`), not by
//! a window of ours. This file owns the Unix socket the library connects to (`$XDG_RUNTIME_DIR/kettle-overlay-
//! <pid>.sock`, mode 0600, named to the game through `KETTLE_OVERLAY_SOCKET`), sends it snapshots of what to show
//! and turns what the user did there into `Command`s. Several game processes may connect; all get the data.
//!
//! [`Host::active`] is true while a library is connected: the menu's "Show overlay" then sends it a toggle
//! instead of opening the in-process window. Games without the library (Gamescope, Windows builds, 32-bit) keep
//! using the in-process window.

#[cfg(all(unix, not(target_os = "macos")))]
pub use imp::Host;
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub use stub::Host;

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod stub {
    use sr_client::Client;
    use sr_model::AppId;
    use std::path::PathBuf;

    pub struct Host;

    impl Host {
        pub fn new(_wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> Host {
            Host
        }
        pub fn sync(&mut self, _want: Option<AppId>) {}
        pub fn active(&self) -> bool {
            false
        }
        pub fn socket(&self) -> Option<PathBuf> {
            None
        }
        pub fn toggle(&mut self, _client: &Client) -> bool {
            false
        }
        pub fn poll(&mut self, _client: &mut Client, _ctx: &eframe::egui::Context) {}
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use sr_client::{Client, Command, OverlaySnapshot, overlay_snapshot};
    use sr_model::{AppId, SteamId};
    use sr_overlay_proto::{
        AchievementRow, FriendRow, FromHook, MAX_LINE, OverlayAction, OverlayData, ToHook, decode, encode,
    };
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    /// Fresh data goes to a visible overlay this often, to a hidden one (for the HUD and the next opening) less.
    const UPDATE_OPEN: Duration = Duration::from_millis(500);
    const UPDATE_CLOSED: Duration = Duration::from_secs(3);

    type Conns = Arc<Mutex<Vec<UnixStream>>>;

    pub struct Host {
        wake: Arc<dyn Fn() + Send + Sync>,
        tx: mpsc::Sender<FromHook>,
        rx: mpsc::Receiver<FromHook>,
        conns: Conns,
        socket: Option<PathBuf>,
        stop: Arc<AtomicBool>,
        started: bool,
        /// The game the data is about.
        app: Option<AppId>,
        visible: bool,
        last_sent: Instant,
        /// A library connected since the last data: send at once.
        fresh: Arc<AtomicBool>,
    }

    fn snapshot_to_data(s: &OverlaySnapshot, hud_on: bool) -> OverlayData {
        OverlayData {
            app: s.app,
            game: s.game.clone(),
            session_secs: s.session_secs,
            total_minutes: 0,
            me: s.me.clone(),
            friends_connected: s.friends_connected,
            friends: s
                .friends
                .iter()
                .map(|f| FriendRow {
                    id: f.id,
                    name: f.name.clone(),
                    status: match (&f.game, f.state) {
                        (Some(g), _) => g.clone(),
                        (None, 0) => "Offline".into(),
                        (None, 3) => "Away".into(),
                        (None, _) => "Online".into(),
                    },
                    online: f.state != 0,
                    in_game: f.game.is_some(),
                })
                .collect(),
            achievements_unlocked: 0,
            achievements_total: 0,
            achievements_recent: Vec::<AchievementRow>::new(),
            hud: hud_on,
            enabled: s.overlay_on,
            animations: s.animations,
        }
    }

    impl Host {
        pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Host {
            let (tx, rx) = mpsc::channel();
            Host {
                wake,
                tx,
                rx,
                conns: Arc::default(),
                socket: None,
                stop: Arc::default(),
                started: false,
                app: None,
                visible: false,
                last_sent: Instant::now(),
                fresh: Arc::default(),
            }
        }

        /// The game the overlay data is about (`None` = no game running). The socket is created the first time
        /// a game runs, so the launch that follows can name it.
        pub fn sync(&mut self, want: Option<AppId>) {
            self.app = want;
            if want.is_some() && !self.started {
                self.start();
            }
            if want.is_none() {
                self.visible = false;
            }
        }

        /// The socket path to give a game (`KETTLE_OVERLAY_SOCKET`); created on first use.
        pub fn socket(&mut self) -> Option<PathBuf> {
            if !self.started {
                self.start();
            }
            self.socket.clone()
        }

        /// Is a game's overlay library connected?
        pub fn active(&self) -> bool {
            self.conns.lock().map(|c| !c.is_empty()).unwrap_or(false)
        }

        fn start(&mut self) {
            self.started = true;
            let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) else {
                eprintln!("[overlay] no XDG_RUNTIME_DIR; the in-game overlay is not available");
                return;
            };
            let path = PathBuf::from(dir).join(format!("kettle-overlay-{}.sock", std::process::id()));
            let _ = std::fs::remove_file(&path);
            let listener = match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[overlay] cannot create {}: {e}", path.display());
                    return;
                }
            };
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            self.socket = Some(path);
            let (conns, tx, wake, stop, fresh) =
                (self.conns.clone(), self.tx.clone(), self.wake.clone(), self.stop.clone(), self.fresh.clone());
            let spawned = std::thread::Builder::new().name("overlay-accept".into()).spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(stream) = stream else { continue };
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(300)));
                    let Ok(reader) = stream.try_clone() else { continue };
                    if let Ok(mut c) = conns.lock() {
                        c.push(stream);
                    }
                    fresh.store(true, Ordering::SeqCst);
                    wake();
                    let (conns, tx, wake) = (conns.clone(), tx.clone(), wake.clone());
                    let _ = std::thread::Builder::new().name("overlay-conn".into()).spawn(move || {
                        let mut r = BufReader::new(reader);
                        let mut buf = Vec::new();
                        loop {
                            buf.clear();
                            match r.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(_) if buf.len() > MAX_LINE => break,
                                Ok(_) => {}
                            }
                            if let Some(m) = decode::<FromHook>(&buf)
                                && tx.send(m).is_ok()
                            {
                                wake();
                            }
                        }
                        // Dropped streams are removed lazily on the next failed write.
                        if let Ok(mut c) = conns.lock() {
                            c.retain(|s| s.peer_addr().is_ok() && s.take_error().ok().flatten().is_none());
                        }
                    });
                }
            });
            if let Err(e) = spawned {
                eprintln!("[overlay] cannot start the socket thread: {e}");
            }
        }

        fn send(&self, msg: &ToHook) -> bool {
            let line = encode(msg);
            let Ok(mut c) = self.conns.lock() else { return false };
            c.retain_mut(|s| s.write_all(&line).is_ok());
            !c.is_empty()
        }

        /// "Show overlay" in the menu. False when no library is connected (the in-process window is used).
        pub fn toggle(&mut self, _client: &Client) -> bool {
            self.active() && self.send(&ToHook::Toggle)
        }

        fn data(&self, client: &Client) -> Option<OverlayData> {
            let app = self.app?;
            let s = overlay_snapshot(client.state(), app, false)?;
            let hud = client.state().settings.hud.preset != sr_steam::mangohud::Preset::Off;
            Some(snapshot_to_data(&s, hud))
        }

        /// Apply what the libraries reported and keep them supplied with data. Call once per frame.
        pub fn poll(&mut self, client: &mut Client, ctx: &eframe::egui::Context) {
            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    FromHook::Hello { .. } => {}
                    FromHook::Visible { open } => self.visible = open,
                    FromHook::Action { a } => match a {
                        OverlayAction::OpenUrl { url } => {
                            if let Err(e) = sr_client::open_url(&url) {
                                eprintln!("[overlay] cannot open {url}: {e:#}");
                            }
                        }
                        OverlayAction::OpenChat { id } => {
                            // Loads the history; the chat panel of the main window stays closed (it is behind the game).
                            client.send(Command::OpenChat(SteamId(id)));
                            client.send(Command::CloseFriends);
                        }
                        OverlayAction::SendChat { id, text } => client.send(Command::SendChat(SteamId(id), text)),
                        OverlayAction::Screenshot | OverlayAction::PerfHud { .. } | OverlayAction::Close => {}
                    },
                }
            }
            if !self.active() {
                return;
            }
            let every = if self.visible { UPDATE_OPEN } else { UPDATE_CLOSED };
            let fresh = self.fresh.swap(false, Ordering::SeqCst);
            if fresh || self.last_sent.elapsed() >= every {
                if let Some(d) = self.data(client) {
                    self.send(&ToHook::Data(d));
                }
                self.last_sent = Instant::now();
            }
            ctx.request_repaint_after(every);
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
            if let Some(p) = self.socket.take() {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}
