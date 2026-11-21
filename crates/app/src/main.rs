//! kettle desktop app: window setup and wiring only.

mod hotkey;
mod overlay_helper;
mod overlay_host;
#[cfg(all(unix, not(target_os = "macos")))]
mod overlay_ipc;
mod tray;

use eframe::egui;
use sr_client::Client;
use sr_ui::{AppUi, TrayGo, theme};
use std::sync::{Arc, mpsc};
use std::time::Instant;

struct App {
    client: Client,
    ui: AppUi,
    started: Instant,
    first_frame_logged: bool,
    timings: bool,
    /// Frames drawn since the last report (a quiet UI draws almost none).
    frames: u32,
    last_report: Instant,
    /// Registers Shift+Tab while a game runs; each press arrives on `toggles`.
    hotkey: hotkey::Hotkey,
    toggles: mpsc::Receiver<()>,
    /// The in-game overlay: a helper process on Linux (see overlay_host.rs), the in-process window otherwise.
    overlay_host: overlay_host::Host,
    /// The tray icon, while "Show Kettle in the tray" is on.
    tray: Option<tray::Tray>,
    /// (library revision, running games) the tray's recent-games list was built from.
    tray_recent_key: (u64, usize),
    /// The window was hidden to the tray by closing it.
    quitting: bool,
    /// "Start minimized" is decided once, when the tray shows up (or after a few seconds without one).
    start_checked: bool,
}

/// Hide the window. Wayland compositors ignore "hide", so there it is minimized.
fn hide_window(ctx: &egui::Context) {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    ctx.send_viewport_cmd(if wayland {
        egui::ViewportCommand::Minimized(true)
    } else {
        egui::ViewportCommand::Visible(false)
    });
}

fn show_window(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

impl App {
    /// Show or hide the overlay: through the helper process when it runs, else the in-process window.
    fn toggle_overlay(&mut self) {
        if !self.overlay_host.toggle(&self.client) {
            self.ui.toggle_overlay(&mut self.client);
        }
    }

    /// Start or stop the tray to match the setting, and handle what its menu asked for.
    fn tray_step(&mut self, ctx: &egui::Context) {
        let iface = self.client.state().settings.interface.clone();
        if iface.tray && self.tray.is_none() {
            let wake_ctx = ctx.clone();
            let icon = app_icon();
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || wake_ctx.request_repaint());
            self.tray = Some(tray::Tray::start(wake, (icon.width, icon.height, icon.rgba)));
        } else if !iface.tray
            && let Some(t) = self.tray.take()
        {
            t.stop();
        }
        let Some(tray) = &self.tray else { return };
        // Recent games for the menu, rebuilt only when the library or the running games changed.
        let key = (self.client.state().library_rev, self.client.state().running.len());
        if key != self.tray_recent_key {
            self.tray_recent_key = key;
            let mut games: Vec<_> =
                self.client.state().games.iter().filter(|g| g.last_played > 0 && g.install.is_installed()).collect();
            games.sort_by_key(|g| std::cmp::Reverse(g.last_played));
            tray.set_recent(games.into_iter().take(5).map(|g| (g.app_id.0, g.name.clone())).collect());
        }
        if !self.start_checked {
            if tray.active() {
                self.start_checked = true;
                if iface.start_minimized || std::env::args().any(|a| a == sr_client::MINIMIZED_ARG) {
                    hide_window(ctx);
                }
            } else if self.started.elapsed().as_secs() >= 6 {
                self.start_checked = true;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs(1));
            }
        }
        while let Some(msg) = tray.try_recv() {
            let go = match msg {
                tray::Msg::Show => None,
                tray::Msg::Store => Some(TrayGo::Store),
                tray::Msg::Library => Some(TrayGo::Library),
                tray::Msg::Friends => Some(TrayGo::Friends),
                tray::Msg::Downloads => Some(TrayGo::Downloads),
                tray::Msg::Settings => Some(TrayGo::Settings),
                tray::Msg::Play(id) => Some(TrayGo::Play(id)),
                tray::Msg::Exit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    continue;
                }
            };
            show_window(ctx);
            if let Some(go) = go {
                self.ui.tray_go(ctx, &mut self.client, go);
            }
        }
    }
}

impl eframe::App for App {
    /// Transparent, so the in-game overlay window can show the game through its dimmed background.
    fn on_exit(&mut self) {
        exit_watchdog();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.tray_step(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            let keep = self.client.state().settings.interface.close_to_tray
                && self.tray.as_ref().is_some_and(tray::Tray::active)
                && !self.quitting
                && !AppUi::quit_requested(&ctx);
            if keep {
                // Closing the window only hides it; Exit in a menu really quits.
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                hide_window(&ctx);
            } else {
                exit_watchdog();
            }
        }
        // The game the overlay belongs to: the one that started first (the same rule as the in-process overlay).
        let game = {
            let s = self.client.state();
            s.running
                .iter()
                .min_by_key(|(app, g)| (g.started, app.0))
                .map(|(app, _)| *app)
                .filter(|_| s.settings.overlay)
        };
        self.hotkey.set_active(game.is_some());
        let in_gamescope = game.is_some_and(|g| self.client.state().running.get(&g).is_some_and(|r| r.gamescope));
        if self.overlay_host.sync(game, in_gamescope) {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.ui.set_overlay_external(self.overlay_host.active());
        while self.toggles.try_recv().is_ok() {
            self.toggle_overlay();
        }
        self.ui.show(ui, &mut self.client);
        // "Games > Show overlay" while the helper is in use arrives as a request.
        if self.ui.take_overlay_request() {
            self.toggle_overlay();
        }
        self.overlay_host.poll(&mut self.client, &ctx);
        if self.timings {
            self.frames += 1;
            if self.last_report.elapsed().as_secs() >= 5 {
                eprintln!("[timing] {} frames in the last 5 s", self.frames);
                (self.frames, self.last_report) = (0, Instant::now());
            }
        }
        if self.timings && !self.first_frame_logged {
            self.first_frame_logged = true;
            eprintln!("[timing] first frame after {:.0} ms", self.started.elapsed().as_secs_f64() * 1e3);
        }
    }
}

/// How long closing may take before the process ends anyway.
const EXIT_LIMIT: std::time::Duration = std::time::Duration::from_millis(1200);

/// Closing must never hang (a background task stuck in a network call, a driver teardown). Once the window
/// is asked to close, this ends the process after [`EXIT_LIMIT`] if the normal shutdown has not finished.
/// Everything the app keeps is written when it changes, so nothing is lost.
fn exit_watchdog() {
    static ARMED: std::sync::Once = std::sync::Once::new();
    ARMED.call_once(|| {
        let _ = std::thread::Builder::new().name("exit-watchdog".into()).spawn(|| {
            std::thread::sleep(EXIT_LIMIT);
            std::process::exit(0);
        });
    });
}

/// The window and task-switcher icon, decoded once from the bundled PNG.
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).expect("bundled icon is a valid PNG");
    let rgba = img.into_rgba8();
    egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
}

fn main() -> eframe::Result {
    // `kettle --overlay ...` is the in-game overlay helper process, not the app (see overlay_helper.rs).
    if let Some(result) = overlay_helper::dispatch() {
        return result;
    }
    let started = Instant::now();
    let timings = std::env::var_os("KETTLE_TIMINGS").is_some();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title(theme::APP_NAME)
            // We draw our own title bar and resize edges (see sr-ui's chrome).
            .with_decorations(false)
            .with_icon(app_icon())
            .with_app_id(theme::APP_ID)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        theme::APP_NAME,
        options,
        Box::new(move |cc| {
            AppUi::setup(&cc.egui_ctx);
            let gpu = cc.wgpu_render_state.as_ref().map(|r| r.adapter.get_info().name).unwrap_or_default();
            let ctx = cc.egui_ctx.clone();
            let (toggle_tx, toggles) = mpsc::channel();
            let toggle_ctx = ctx.clone();
            let hotkey = hotkey::Hotkey::new(Arc::new(move || {
                let _ = toggle_tx.send(());
                toggle_ctx.request_repaint();
            }));
            let host_ctx = ctx.clone();
            let overlay_host = overlay_host::Host::new(Arc::new(move || host_ctx.request_repaint()));
            let t = Instant::now();
            let mut client = Client::new(Arc::new(move || ctx.request_repaint()))?;
            // `kettle --play <appid>` (desktop and menu shortcuts): start that game once the library is scanned.
            if let Some(id) = play_arg(std::env::args().skip(1)) {
                client.send(sr_client::Command::Manage(sr_client::ManageCmd::PlayWhenReady(id)));
            }
            if timings {
                let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
                eprintln!("[timing] window+GPU ready {:.0} ms, client {:.0} ms", ms(t - started), ms(t.elapsed()));
            }
            let mut ui = AppUi::default();
            ui.set_gpu_name(gpu);
            Ok(Box::new(App {
                client,
                ui,
                started,
                first_frame_logged: false,
                timings,
                frames: 0,
                last_report: Instant::now(),
                hotkey,
                toggles,
                overlay_host,
                tray: None,
                tray_recent_key: (u64::MAX, 0),
                quitting: false,
                start_checked: false,
            }))
        }),
    )
}

/// The app id after `--play` (or in `--play=<id>`), when given.
fn play_arg(mut args: impl Iterator<Item = String>) -> Option<u32> {
    while let Some(a) = args.next() {
        if a == "--play" {
            return args.next()?.parse().ok();
        }
        if let Some(v) = a.strip_prefix("--play=") {
            return v.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::play_arg;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter().map(|s| s.to_string()).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn finds_the_play_argument() {
        assert_eq!(play_arg(args(&["--play", "220"])), Some(220));
        assert_eq!(play_arg(args(&["--play=7"])), Some(7));
        assert_eq!(play_arg(args(&["--other", "--play", "5"])), Some(5));
        assert_eq!(play_arg(args(&["--play", "abc"])), None);
        assert_eq!(play_arg(args(&["--play"])), None);
        assert_eq!(play_arg(args(&[])), None);
    }
}
