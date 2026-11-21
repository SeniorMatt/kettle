//! The in-game overlay process: `kettle --overlay <appid> --socket <path>`.
//!
//! One transparent, undecorated, override-redirect X11 window the size of the primary monitor. Override-redirect
//! windows are not managed by the window manager: the desktop plays no open or close animation, shows no taskbar
//! entry, never changes focus on its own and so never minimizes or un-fullscreens the game underneath. The window
//! is hidden (unmapped) until the main app sends `show`; while shown it takes the keyboard (focus plus an
//! `XGrabKeyboard`, released on hide) so typing does not reach the game. It has no Steam connection: everything
//! it shows comes from the main app over the socket (see `sr_client::overlay_proto`), and what the user does goes
//! back the same way. Drawing is `sr_ui::OverlayPanel`.
//!
//! Inside Gamescope (`--gamescope`, the game runs on Gamescope's private X server and `DISPLAY` points there)
//! the window is a normal one instead, with the properties Gamescope looks for: `STEAM_OVERLAY` = 1 makes it
//! the overlay layer drawn above the game, `STEAM_INPUT_FOCUS` = 1 while shown sends keyboard and mouse to it
//! (0 when hidden), and `_NET_WM_WINDOW_OPACITY` is 0 while hidden (Gamescope skips an overlay with no opacity).
//! They are set before the window is first mapped, so Gamescope never takes it for the game.
//!
//! Not done on purpose: the pointer is not grabbed. A game that has captured the mouse itself (relative mode)
//! may keep it; clicks then do not reach the overlay until it is released.

#[cfg(all(unix, not(target_os = "macos")))]
pub use imp::dispatch;

/// No helper process on this system; the in-process overlay of `sr-ui` is used.
#[cfg(not(all(unix, not(target_os = "macos"))))]
pub fn dispatch() -> Option<eframe::Result> {
    None
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    use crate::overlay_ipc;
    use eframe::egui::{self, Key, Modifiers, ViewportCommand};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use sr_client::{FromHelper, ToHelper};
    use sr_ui::{AppUi, OverlayPanel};
    use std::io::BufReader;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::protocol::xproto::{
        AtomEnum, ConfigureWindowAux, ConnectionExt as _, GrabMode, GrabStatus, PropMode, StackMode,
    };
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// Frames spent hiding the window after startup (eframe maps it once after its first frame).
    const INIT_FRAMES: u32 = 4;
    /// Tries to take the keyboard after showing (the window has to be mapped first).
    const GRAB_TRIES: u32 = 40;

    /// `Some(result)` when this process was started as the overlay helper; the caller returns it from `main`.
    pub fn dispatch() -> Option<eframe::Result> {
        let args: Vec<String> = std::env::args().collect();
        let at = args.iter().position(|a| a == "--overlay")?;
        let socket = args.iter().position(|a| a == "--socket").and_then(|i| args.get(i + 1)).map(PathBuf::from);
        let (Some(_app), Some(socket)) = (args.get(at + 1).and_then(|a| a.parse::<u32>().ok()), socket) else {
            eprintln!("usage: kettle --overlay <appid> --socket <path>");
            return Some(Ok(()));
        };
        Some(run(socket, args.iter().any(|a| a == "--gamescope")))
    }

    /// Position and size of the primary monitor in pixels (the whole screen when RandR does not say).
    fn monitor_geometry() -> Option<(i32, i32, u32, u32)> {
        let (conn, n) = RustConnection::connect(None).ok()?;
        let screen = &conn.setup().roots[n];
        let whole = (0, 0, u32::from(screen.width_in_pixels), u32::from(screen.height_in_pixels));
        let reply = conn.randr_get_monitors(screen.root, true).ok().and_then(|c| c.reply().ok());
        let primary = reply.and_then(|r| {
            let m = r.monitors.iter().find(|m| m.primary).or(r.monitors.first())?;
            Some((i32::from(m.x), i32::from(m.y), u32::from(m.width), u32::from(m.height)))
        });
        Some(primary.unwrap_or(whole))
    }

    /// Set the properties Gamescope reads (see the module comment). `shown` = the overlay is up.
    fn gamescope_props(conn: &RustConnection, win: u32, shown: bool) {
        let atom = |name: &[u8]| conn.intern_atom(false, name).ok().and_then(|c| c.reply().ok()).map(|r| r.atom);
        let set = |name: &[u8], value: u32| {
            if let Some(a) = atom(name) {
                let _ = conn.change_property32(PropMode::REPLACE, win, a, AtomEnum::CARDINAL, &[value]);
            }
        };
        set(b"STEAM_OVERLAY", 1);
        set(b"STEAM_INPUT_FOCUS", u32::from(shown));
        set(b"_NET_WM_WINDOW_OPACITY", if shown { u32::MAX } else { 0 });
        let _ = conn.flush();
    }

    fn run(socket: PathBuf, gamescope: bool) -> eframe::Result {
        let geom = monitor_geometry().unwrap_or((0, 0, 1920, 1080));
        let mut viewport = egui::ViewportBuilder::default()
            .with_title("Kettle overlay")
            .with_decorations(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_position([geom.0 as f32, geom.1 as f32])
            .with_inner_size([geom.2 as f32, geom.3 as f32])
            .with_visible(false);
        if !gamescope {
            // On the desktop: invisible to the window manager (no animation, no focus change, no minimizing).
            viewport =
                viewport.with_override_redirect(true).with_window_type(egui::viewport::X11WindowType::Notification);
        }
        let options = eframe::NativeOptions { renderer: eframe::Renderer::Wgpu, viewport, ..Default::default() };
        eframe::run_native(
            "Kettle overlay",
            options,
            Box::new(move |cc| {
                AppUi::setup(&cc.egui_ctx);
                let out = UnixStream::connect(&socket)?;
                out.set_write_timeout(Some(Duration::from_millis(500)))?;
                let (tx, rx) = mpsc::channel();
                let reader = out.try_clone()?;
                let ctx = cc.egui_ctx.clone();
                std::thread::Builder::new().name("overlay-reader".into()).spawn(move || {
                    let mut reader = BufReader::new(reader);
                    let mut buf = Vec::new();
                    while let Ok(Some(())) = overlay_ipc::read_line(&mut reader, &mut buf) {
                        if let Some(msg) = overlay_ipc::decode::<ToHelper>(&buf) {
                            if tx.send(msg).is_err() {
                                return;
                            }
                            ctx.request_repaint();
                        }
                    }
                    // The main app is gone (or hung up): end with it.
                    let _ = tx.send(ToHelper::Quit);
                    ctx.request_repaint();
                })?;
                let conn = RustConnection::connect(None).ok().map(|(c, _)| c);
                let win = window_id(cc);
                if gamescope && let (Some(conn), Some(win)) = (&conn, win) {
                    gamescope_props(conn, win, false);
                }
                Ok(Box::new(Helper {
                    gamescope,
                    win,
                    panel: OverlayPanel::default(),
                    rx,
                    out,
                    conn,
                    geom,
                    visible: false,
                    frames: 0,
                    ready_sent: false,
                    grab_tries: 0,
                    placed: false,
                }))
            }),
        )
    }

    struct Helper {
        /// Running on Gamescope's X server.
        gamescope: bool,
        /// Our X window.
        win: Option<u32>,
        panel: OverlayPanel,
        rx: mpsc::Receiver<ToHelper>,
        out: UnixStream,
        conn: Option<RustConnection>,
        geom: (i32, i32, u32, u32),
        visible: bool,
        frames: u32,
        ready_sent: bool,
        /// Tries left to take the keyboard after a show.
        grab_tries: u32,
        placed: bool,
    }

    /// The X window of the egui window.
    fn window_id(frame: &impl HasWindowHandle) -> Option<u32> {
        match frame.window_handle().ok()?.as_raw() {
            RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok(),
            RawWindowHandle::Xcb(h) => Some(h.window.get()),
            _ => None,
        }
    }

    impl Helper {
        fn say(&mut self, msg: &FromHelper) {
            let _ = overlay_ipc::send(&mut self.out, msg);
        }

        fn show(&mut self, ctx: &egui::Context, snap: sr_client::OverlaySnapshot) {
            self.panel.on_show(snap);
            self.visible = true;
            self.placed = false;
            self.grab_tries = GRAB_TRIES;
            self.gamescope_shown(true);
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.request_repaint();
        }

        fn hide(&mut self, ctx: &egui::Context) {
            for m in self.panel.flush() {
                self.say(&m);
            }
            self.visible = false;
            self.grab_tries = 0;
            if let Some(conn) = &self.conn {
                let _ = conn.ungrab_keyboard(x11rb::CURRENT_TIME);
                let _ = conn.flush();
            }
            self.gamescope_shown(false);
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        /// Inside Gamescope: tell it the overlay is up (it takes the input) or gone.
        fn gamescope_shown(&self, shown: bool) {
            if let (true, Some(conn), Some(win)) = (self.gamescope, &self.conn, self.win) {
                gamescope_props(conn, win, shown);
            }
        }

        /// Cover the monitor exactly, stay on top, and take the keyboard. Retried until the window is mapped.
        fn take_input(&mut self, frame: &eframe::Frame, ctx: &egui::Context) {
            let (Some(win), Some(conn)) = (window_id(frame), &self.conn) else {
                self.grab_tries = 0;
                return;
            };
            if !self.placed {
                let g = self.geom;
                let aux = ConfigureWindowAux::new().x(g.0).y(g.1).width(g.2).height(g.3).stack_mode(StackMode::ABOVE);
                let _ = conn.configure_window(win, &aux);
                self.placed = true;
            }
            let grabbed = conn
                .grab_keyboard(true, win, x11rb::CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)
                .ok()
                .and_then(|c| c.reply().ok())
                .is_some_and(|r| r.status == GrabStatus::SUCCESS);
            if grabbed {
                // Keyboard grab only, never XSetInputFocus: a real focus change makes fullscreen games
                // (SDL, Wine) minimize themselves; a grab gives them FocusOut with mode NotifyGrab, which they
                // ignore, and the keys still reach us because the grab window gets every key event.
                self.grab_tries = 0;
            } else {
                self.grab_tries -= 1;
                ctx.request_repaint_after(Duration::from_millis(30));
            }
            let _ = conn.flush();
        }
    }

    impl eframe::App for Helper {
        fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
            egui::Rgba::TRANSPARENT.to_array()
        }

        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            if self.win.is_none() {
                self.win = window_id(&*frame);
            }
            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    ToHelper::Show(s) => self.show(&ctx, s),
                    ToHelper::Update(s) => self.panel.update(s),
                    ToHelper::Hide => self.hide(&ctx),
                    ToHelper::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
                }
            }
            if !self.visible {
                // eframe maps the window once after its first frame; hide it again, then say we are ready.
                if self.frames < INIT_FRAMES {
                    self.frames += 1;
                    ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                    ctx.request_repaint();
                } else if !self.ready_sent {
                    self.ready_sent = true;
                    self.say(&FromHelper::Ready);
                }
                return;
            }
            if self.grab_tries > 0 {
                self.take_input(frame, &ctx);
            }
            // Without the global shortcut (plain X11) Shift+Tab reaches us as a key press.
            let close_key = ui.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::Tab));
            let out = self.panel.show(ui);
            for m in &out.send {
                self.say(m);
            }
            if out.close || close_key {
                self.hide(&ctx);
                self.say(&FromHelper::Hidden);
            }
        }
    }
}
