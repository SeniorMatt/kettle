//! The main app's side of the in-game overlay.
//!
//! The overlay is drawn by a second process, `kettle --overlay <appid> --socket <path>` (see `overlay_helper`),
//! started when the first game starts and ended when the game ends. It is forced onto X11 (even on Wayland, where
//! games under Proton are XWayland windows) so it can be an override-redirect window: the desktop plays no
//! window animation for it and never changes focus or minimizes the game. This file spawns and stops that
//! process, owns the Unix socket, sends it snapshots and turns what the user did there into `Command`s.
//!
//! If the helper cannot start (no X display, no runtime folder, spawn error) or dies, [`Host::active`] is false and
//! the caller falls back to the in-process overlay window of `sr-ui`.

#[cfg(all(unix, not(target_os = "macos")))]
pub use imp::Host;
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub use stub::Host;

#[cfg(not(all(unix, not(target_os = "macos"))))]
mod stub {
    use sr_client::Client;
    use sr_model::AppId;

    /// No helper on this system: the in-process overlay is always used.
    pub struct Host;

    impl Host {
        pub fn new(_wake: std::sync::Arc<dyn Fn() + Send + Sync>) -> Host {
            Host
        }
        pub fn sync(&mut self, _want: Option<AppId>, _gamescope: bool) -> bool {
            false
        }
        pub fn active(&self) -> bool {
            false
        }
        pub fn toggle(&mut self, _client: &Client) -> bool {
            false
        }
        pub fn poll(&mut self, _client: &mut Client, _ctx: &eframe::egui::Context) {}
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use crate::overlay_ipc;
    use sr_client::{Client, Command, FromHelper, OverlaySnapshot, ToHelper, detect_hud, overlay_snapshot};
    use sr_model::{AppId, SteamId};
    use std::io::BufReader;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::process::{Child, Command as Process, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    /// How long a game in Gamescope has to report its display before the normal overlay is used instead.
    const DISPLAY_LIMIT: Duration = Duration::from_secs(90);
    /// How long the helper has to connect before we give up on it.
    const CONNECT_LIMIT: Duration = Duration::from_secs(20);
    /// A toggle this soon after the user closed the overlay itself is the same key press seen twice
    /// (by the helper's keyboard grab and by the global shortcut).
    const DEBOUNCE: Duration = Duration::from_millis(300);
    /// Fresh data goes to a shown overlay at most this often.
    const UPDATE_EVERY: Duration = Duration::from_millis(250);

    type Shared = Arc<Mutex<Option<UnixStream>>>;

    pub struct Host {
        wake: Arc<dyn Fn() + Send + Sync>,
        tx: mpsc::Sender<FromHelper>,
        rx: mpsc::Receiver<FromHelper>,
        /// The helper process and the game it was started for.
        child: Option<(Child, AppId)>,
        conn: Shared,
        stop_accept: Arc<AtomicBool>,
        socket: Option<PathBuf>,
        /// Set once the helper failed; the in-process overlay is used from then on (until the game changes).
        failed: bool,
        ready: bool,
        visible: bool,
        pending_show: bool,
        last_hidden: Option<Instant>,
        last_sent: Instant,
        dirty: bool,
        detect: Option<(Instant, bool)>,
        /// A game in Gamescope is running but has not reported its X display yet.
        waiting: bool,
        /// Since when the running game has been waited for.
        waiting_since: Option<Instant>,
    }

    impl Host {
        pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Host {
            let (tx, rx) = mpsc::channel();
            Host {
                wake,
                tx,
                rx,
                child: None,
                conn: Arc::default(),
                stop_accept: Arc::default(),
                socket: None,
                failed: false,
                ready: false,
                visible: false,
                pending_show: false,
                last_hidden: None,
                last_sent: Instant::now(),
                dirty: false,
                detect: None,
                waiting: false,
                waiting_since: None,
            }
        }

        /// Keep the helper in step with the game: started for `want`, ended when there is none. `gamescope` says
        /// the game runs inside Gamescope: the helper then joins Gamescope's own X server, whose name the game's
        /// launcher reports in a file a moment after the start. Returns true while that is awaited, so the
        /// caller asks for another frame soon.
        pub fn sync(&mut self, want: Option<AppId>, gamescope: bool) -> bool {
            self.waiting = false;
            let died = self.child.as_mut().is_some_and(|(c, _)| matches!(c.try_wait(), Ok(Some(_))));
            if died {
                eprintln!("[overlay] the helper process ended; using the built-in overlay window");
                self.stop();
                self.failed = true;
            }
            match (want, self.child.as_ref().map(|(_, a)| *a)) {
                (None, _) => {
                    if self.child.is_some() {
                        self.stop();
                    }
                    self.failed = false;
                    self.waiting_since = None;
                }
                (Some(app), Some(running)) if app != running => {
                    self.stop();
                    self.failed = false;
                    self.waiting_since = None;
                    self.start(app, gamescope);
                }
                (Some(app), None) if !self.failed => self.start(app, gamescope),
                _ => {}
            }
            self.waiting
        }

        /// Is the helper (started or still connecting) the one that shows the overlay?
        pub fn active(&self) -> bool {
            (self.child.is_some() && !self.failed) || self.waiting
        }

        fn start(&mut self, app: AppId, gamescope: bool) {
            // A game in Gamescope lives on Gamescope's own X server; its name arrives from the launcher shim.
            let mut display = None;
            if gamescope {
                match sr_client::gamescope_display(app) {
                    Some(d) => display = Some(d),
                    None => {
                        let since = *self.waiting_since.get_or_insert_with(Instant::now);
                        if since.elapsed() > DISPLAY_LIMIT {
                            eprintln!("[overlay] the game's Gamescope display never appeared; using the built-in overlay");
                            self.failed = true;
                        } else {
                            self.waiting = true;
                        }
                        return;
                    }
                }
            }
            let Some(path) = overlay_ipc::socket_path(std::process::id()) else {
                eprintln!("[overlay] no XDG_RUNTIME_DIR; using the built-in overlay window");
                self.failed = true;
                return;
            };
            if display.is_none() && std::env::var_os("DISPLAY").is_none_or(|d| d.is_empty()) {
                eprintln!("[overlay] no X11 display (XWayland); using the built-in overlay window");
                self.failed = true;
                return;
            }
            let _ = std::fs::remove_file(&path);
            let listener = match UnixListener::bind(&path) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("[overlay] cannot create {}: {e}", path.display());
                    self.failed = true;
                    return;
                }
            };
            // The runtime folder is already private; this makes the socket itself private too.
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            let exe = match std::env::current_exe() {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("[overlay] cannot find our own program: {e}");
                    let _ = std::fs::remove_file(&path);
                    self.failed = true;
                    return;
                }
            };
            // Wayland is hidden from the helper, so winit picks X11 (XWayland on a Wayland desktop).
            let mut cmd = Process::new(exe);
            cmd.arg("--overlay").arg(app.0.to_string()).arg("--socket").arg(&path);
            if let Some(d) = &display {
                // Inside Gamescope's X server: a normal window with the properties Gamescope draws on top.
                cmd.arg("--gamescope").env("DISPLAY", d);
            }
            let child = cmd
                .env_remove("WAYLAND_DISPLAY")
                .env("XDG_SESSION_TYPE", "x11")
                .stdin(Stdio::null())
                .spawn();
            match child {
                Ok(c) => self.child = Some((c, app)),
                Err(e) => {
                    eprintln!("[overlay] cannot start the helper: {e}");
                    let _ = std::fs::remove_file(&path);
                    self.failed = true;
                    return;
                }
            }
            self.socket = Some(path);
            self.stop_accept = Arc::default();
            self.accept(listener);
        }

        /// A thread waits for the helper to connect, then reads what it sends until it hangs up.
        fn accept(&self, listener: UnixListener) {
            let (conn, tx, wake, stop) =
                (self.conn.clone(), self.tx.clone(), self.wake.clone(), self.stop_accept.clone());
            let _ = listener.set_nonblocking(true);
            let spawned = std::thread::Builder::new().name("overlay-accept".into()).spawn(move || {
                let begin = Instant::now();
                let stream = loop {
                    if stop.load(Ordering::SeqCst) || begin.elapsed() > CONNECT_LIMIT {
                        return;
                    }
                    match listener.accept() {
                        Ok((s, _)) => break s,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(50))
                        }
                        Err(_) => return,
                    }
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
                let Ok(reader) = stream.try_clone() else { return };
                if let Ok(mut c) = conn.lock() {
                    *c = Some(stream);
                }
                let mut reader = BufReader::new(reader);
                let mut buf = Vec::new();
                while let Ok(Some(())) = overlay_ipc::read_line(&mut reader, &mut buf) {
                    if let Some(msg) = overlay_ipc::decode::<FromHelper>(&buf) {
                        if tx.send(msg).is_err() {
                            return;
                        }
                        wake();
                    }
                }
                if let Ok(mut c) = conn.lock() {
                    *c = None;
                }
            });
            if let Err(e) = spawned {
                eprintln!("[overlay] cannot start the socket thread: {e}");
            }
        }

        fn send(&mut self, msg: &ToHelper) -> bool {
            let Ok(mut guard) = self.conn.lock() else { return false };
            let Some(stream) = guard.as_mut() else { return false };
            if overlay_ipc::send(stream, msg).is_err() {
                *guard = None;
                return false;
            }
            self.last_sent = Instant::now();
            true
        }

        fn stop(&mut self) {
            if self.child.is_some() {
                self.send(&ToHelper::Quit);
            }
            self.stop_accept.store(true, Ordering::SeqCst);
            if let Ok(mut c) = self.conn.lock() {
                *c = None;
            }
            if let Some((mut child, _)) = self.child.take() {
                // Our own child process only. Quit above normally ends it first.
                let _ = child.kill();
                let _ = child.wait();
            }
            if let Some(path) = self.socket.take() {
                let _ = std::fs::remove_file(path);
            }
            self.ready = false;
            self.visible = false;
            self.pending_show = false;
            self.dirty = false;
        }

        fn hud_installed(&mut self) -> bool {
            if let Some((at, v)) = self.detect
                && at.elapsed() < Duration::from_secs(5)
            {
                return v;
            }
            let v = detect_hud().installed();
            self.detect = Some((Instant::now(), v));
            v
        }

        fn snapshot(&mut self, client: &Client) -> Option<OverlaySnapshot> {
            let app = self.child.as_ref().map(|(_, a)| *a)?;
            let installed = self.hud_installed();
            overlay_snapshot(client.state(), app, installed)
        }

        /// The shortcut was pressed (or "Show overlay" chosen). False when the helper is not in use, so the
        /// caller shows the built-in window instead.
        pub fn toggle(&mut self, client: &Client) -> bool {
            if !self.active() {
                return false;
            }
            if self.child.is_none() {
                return true; // the game's display is not known yet; the key press is dropped
            }
            if self.last_hidden.is_some_and(|t| t.elapsed() < DEBOUNCE) {
                return true;
            }
            if self.visible {
                self.visible = false;
                self.send(&ToHelper::Hide);
            } else if let Some(s) = self.snapshot(client) {
                if self.ready && self.send(&ToHelper::Show(s)) {
                    self.visible = true;
                } else {
                    // Still starting: shown as soon as it says it is ready.
                    self.pending_show = true;
                }
            }
            true
        }

        /// Apply what the helper reported and keep a shown overlay up to date. Call once per frame.
        pub fn poll(&mut self, client: &mut Client, ctx: &eframe::egui::Context) {
            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    FromHelper::Ready => {
                        self.ready = true;
                        if self.pending_show {
                            self.pending_show = false;
                            if let Some(s) = self.snapshot(client) {
                                self.visible = self.send(&ToHelper::Show(s));
                            }
                        }
                    }
                    FromHelper::Hidden => {
                        self.visible = false;
                        self.last_hidden = Some(Instant::now());
                    }
                    FromHelper::OpenChat { id } => {
                        // Loads the history. The chat panel in the main window is closed again: it is behind the game.
                        client.send(Command::OpenChat(SteamId(id)));
                        client.send(Command::CloseFriends);
                    }
                    FromHelper::SendChat { id, text } => client.send(Command::SendChat(SteamId(id), text)),
                    FromHelper::SetNote { app, text } => client.send(Command::SetNote(AppId(app), text)),
                    FromHelper::SetOverlay { on } => client.send(Command::SetOverlay(on)),
                    FromHelper::SetHud(h) => client.send(Command::SetHud(h)),
                }
            }
            if self.visible {
                if self.last_sent.elapsed() >= UPDATE_EVERY {
                    self.dirty = false;
                    if let Some(s) = self.snapshot(client) {
                        self.send(&ToHelper::Update(s));
                    }
                } else {
                    self.dirty = true;
                    ctx.request_repaint_after(UPDATE_EVERY);
                }
            }
        }
    }

    impl Drop for Host {
        fn drop(&mut self) {
            self.stop();
        }
    }
}
