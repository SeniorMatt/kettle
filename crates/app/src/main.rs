//! steam-rust desktop app: window setup and wiring only.

use eframe::egui;
use sr_client::Client;
use sr_ui::{AppUi, theme};
use std::sync::Arc;
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
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui.show(ui, &mut self.client);
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

/// The window and task-switcher icon, decoded once from the bundled PNG.
fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../assets/icon.png")).expect("bundled icon is a valid PNG");
    let rgba = img.into_rgba8();
    egui::IconData { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() }
}

fn main() -> eframe::Result {
    let started = Instant::now();
    let timings = std::env::var_os("STEAM_RUST_TIMINGS").is_some();
    let options = eframe::NativeOptions {
        renderer: eframe::Renderer::Wgpu,
        viewport: egui::ViewportBuilder::default()
            .with_title(theme::APP_NAME)
            // We draw our own title bar and resize edges (see sr-ui's chrome).
            .with_decorations(false)
            .with_icon(app_icon())
            .with_app_id(theme::APP_NAME)
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        theme::APP_NAME,
        options,
        Box::new(move |cc| {
            AppUi::setup(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let t = Instant::now();
            let client = Client::new(Arc::new(move || ctx.request_repaint()))?;
            if timings {
                let ms = |d: std::time::Duration| d.as_secs_f64() * 1e3;
                eprintln!("[timing] window+GPU ready {:.0} ms, client {:.0} ms", ms(t - started), ms(t.elapsed()));
            }
            Ok(Box::new(App {
                client,
                ui: AppUi::default(),
                started,
                first_frame_logged: false,
                timings,
                frames: 0,
                last_report: Instant::now(),
            }))
        }),
    )
}
