//! The Downloads page, laid out like Steam's: the running download on top (big art, percent, a speed graph
//! of the last minute, peak, total and time left), then "UP NEXT" with up/down buttons, "UNSCHEDULED"
//! (paused), and "COMPLETED". The page repaints only when new progress arrives (about four times a second
//! while downloading) and not at all when idle.

use crate::textures::{Art, Textures};
use crate::widgets::{self, ButtonStyle};
use crate::{Nav, anim, fmt, theme};
use egui::{Color32, Id, Pos2, RichText, Sense, Stroke, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{
    ActiveDownload, Client, Command, DownloadItem, DownloadPhase, FinishedDownload, ManageCmd, Outcome, PausedDownload,
    SpeedHistory, StopKind, rate_text,
};
use sr_model::{AppId, ArtKind};

/// How fast the bar glides toward the newest number (per second).
const GLIDE: f32 = 4.0;
const THUMB: Vec2 = Vec2::new(150.0, 70.0);
const HERO_H: f32 = 130.0;
const GRAPH_H: f32 = 64.0;
const NETWORK: Color32 = theme::ACCENT;
const DISK: Color32 = theme::VERIFY;

pub fn show(ui: &mut Ui, client: &mut Client, tex: &mut Textures, nav: &mut Option<Nav>) {
    let state = client.state().downloads.clone();
    let bits = client.state().settings.downloads.bits_per_second;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(32, 24)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let empty = state.active.is_none()
                && state.queue.is_empty()
                && state.paused.is_empty()
                && state.finished.is_empty();
            if empty {
                empty_page(ui);
                return;
            }
            header(ui, client, &state);
            if let Some(a) = &state.active {
                active_card(ui, client, tex, a, &state.speed, bits, nav);
            }
            if !state.queue.is_empty() {
                ui.add_space(14.0);
                widgets::section_title(ui, &format!("UP NEXT ({})", state.queue.len()));
                let last = state.queue.len() - 1;
                for (i, item) in state.queue.iter().enumerate() {
                    queued_row(ui, client, tex, nav, item, i, last);
                }
            }
            if !state.paused.is_empty() {
                ui.add_space(14.0);
                widgets::section_title(ui, &format!("UNSCHEDULED ({})", state.paused.len()));
                for p in &state.paused {
                    paused_row(ui, client, tex, nav, p);
                }
            }
            if !state.finished.is_empty() {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    widgets::section_title(ui, &format!("COMPLETED ({})", state.finished.len()));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::link(ui, "Clear").clicked() {
                            client.send(Command::ClearFinishedDownloads);
                        }
                    });
                });
                for f in &state.finished {
                    finished_row(ui, client, tex, nav, f);
                }
            }
        });
    });
}

fn empty_page(ui: &mut Ui) {
    ui.add_space(60.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(icon::DOWNLOAD_SIMPLE).font(theme::regular(40.0)).color(theme::TEXT_UNINSTALLED));
        ui.add_space(8.0);
        ui.label(RichText::new("Nothing is downloading").font(theme::medium(18.0)).color(theme::TEXT));
        ui.label(RichText::new("Install a game from its page in your library.").color(theme::TEXT_DIM));
    });
}

/// "Pause all" / "Resume all" and the note when a running game holds the downloads.
fn header(ui: &mut Ui, client: &mut Client, state: &sr_client::DownloadsState) {
    let anything_running = state.active.as_ref().is_some_and(|a| a.stopping.is_none()) || !state.queue.is_empty();
    ui.horizontal(|ui| {
        if state.gameplay_paused {
            ui.label(
                RichText::new(format!("{}  Waiting for the game to close (Settings > Downloads)", icon::PAUSE_CIRCLE))
                    .color(theme::TEXT_DIM),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if anything_running {
                if widgets::button(ui, "Pause all", ButtonStyle::Secondary, true).clicked() {
                    client.send(Command::Manage(ManageCmd::PauseAllDownloads));
                }
            } else if !state.paused.is_empty()
                && widgets::button(ui, "Resume all", ButtonStyle::Primary, true).clicked()
            {
                client.send(Command::Manage(ManageCmd::ResumeAllDownloads));
            }
        });
    });
}

fn thumb(ui: &mut Ui, client: &Client, tex: &mut Textures, app: AppId, size: Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    match tex.get(client, app, ArtKind::Header) {
        Art::Ready(t, a) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(a)),
        Art::Loading => widgets::placeholder(ui, rect),
        Art::Missing => {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
        }
    }
}

fn phase_text(a: &ActiveDownload, shown: f32) -> String {
    if let Some(s) = a.stopping {
        return if s == StopKind::Pause { "PAUSING".into() } else { "CANCELLING".into() };
    }
    let pct = (shown * 100.0).round() as u32;
    match a.phase {
        DownloadPhase::Preparing => "PREPARING".into(),
        DownloadPhase::Downloading => format!("DOWNLOADING {pct}%"),
        DownloadPhase::Verifying => format!("VERIFYING {pct}%"),
        DownloadPhase::Finished => "FINISHING".into(),
        DownloadPhase::Failed => "FAILED".into(),
    }
}

fn active_card(
    ui: &mut Ui,
    client: &mut Client,
    tex: &mut Textures,
    a: &ActiveDownload,
    speed: &SpeedHistory,
    bits: bool,
    nav: &mut Option<Nav>,
) {
    let req = &a.item.request;
    let frame = egui::Frame::new().fill(theme::BG_CARD).corner_radius(theme::RADIUS);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        // Hero art with the name and the state laid over a darkening gradient.
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), HERO_H), Sense::click());
        let shown = anim::glide(ui.ctx(), Id::new(("dl-bar", req.app.0)), a.fraction(), GLIDE);
        match tex.get(client, req.app, ArtKind::Hero) {
            Art::Ready(t, al) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(al)),
            _ => match tex.get(client, req.app, ArtKind::Header) {
                Art::Ready(t, al) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(al)),
                _ => {
                    ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
                }
            },
        }
        widgets::vgradient(ui, rect, Color32::TRANSPARENT, theme::BG_CARD);
        let pad = 16.0;
        ui.painter().text(
            Pos2::new(rect.left() + pad, rect.bottom() - 42.0),
            egui::Align2::LEFT_BOTTOM,
            &req.name,
            theme::bold(22.0),
            Color32::WHITE,
        );
        let sub = format!("{}  |  {}", a.item.kind.label(), req.library.display());
        ui.painter().text(
            Pos2::new(rect.left() + pad, rect.bottom() - 22.0),
            egui::Align2::LEFT_BOTTOM,
            sub,
            theme::regular(12.0),
            theme::TEXT,
        );
        ui.painter().text(
            Pos2::new(rect.right() - pad, rect.bottom() - 22.0),
            egui::Align2::RIGHT_BOTTOM,
            phase_text(a, shown),
            theme::bold(13.0),
            theme::ACCENT,
        );
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            *nav = Some(Nav::Game(req.app));
        }

        egui::Frame::new().inner_margin(egui::Margin::symmetric(14, 8)).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 8.0;
            let fill = if a.phase == DownloadPhase::Verifying { theme::VERIFY } else { theme::ACCENT };
            widgets::progress_bar(ui, ui.available_width(), 6.0, shown, fill);
            graph(ui, speed);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 28.0;
                stat(ui, "SPEED", &rate_text(a.bytes_per_sec, bits));
                stat(ui, "PEAK", &rate_text(a.peak_bytes_per_sec, bits));
                stat(ui, "DOWNLOADED", &fmt::bytes(a.network_bytes));
                stat(ui, "PROGRESS", &format!("{} of {}", fmt::bytes(a.done_bytes), fmt::bytes(a.total_bytes)));
                stat(ui, "TIME LEFT", &a.eta_secs().map_or("-".into(), fmt::duration));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    controls(ui, client, a);
                });
            });
            if let Some(m) = &a.message {
                ui.label(RichText::new(m).color(theme::TEXT_DIM).small());
            }
        });
    });
}

/// The speed of the last minute: network (blue) and disk (green), scaled to the fastest second.
fn graph(ui: &mut Ui, speed: &SpeedHistory) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), GRAPH_H), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, theme::RADIUS, theme::BG_TOP);
    for k in 1..4 {
        let y = rect.bottom() - rect.height() * k as f32 / 4.0;
        p.hline(rect.x_range(), y, Stroke::new(1.0, theme::SETTINGS_SEP.gamma_multiply(0.5)));
    }
    let top = speed.max().max(1_000_000.0);
    let n = sr_client::SPEED_SAMPLES;
    let step = rect.width() / (n - 1) as f32;
    // The newest sample sits at the right edge; a short history starts part-way in.
    let skipped = n - speed.samples.len();
    let line = |pick: fn(&(f32, f32)) -> f32| -> Vec<Pos2> {
        speed
            .samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let x = rect.left() + step * (skipped + i) as f32;
                Pos2::new(x, rect.bottom() - 2.0 - (rect.height() - 4.0) * (pick(s) / top).clamp(0.0, 1.0))
            })
            .collect()
    };
    if speed.samples.len() >= 2 {
        p.add(egui::Shape::line(line(|s| s.1), Stroke::new(1.5, DISK)));
        p.add(egui::Shape::line(line(|s| s.0), Stroke::new(1.5, NETWORK)));
    }
    p.text(
        Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        format!("{}  last 60 s", fmt::speed(top as f64)),
        theme::regular(10.5),
        theme::TEXT_DIM,
    );
    let key = |x: f32, color: Color32, label: &str| {
        p.circle_filled(Pos2::new(rect.right() - x, rect.top() + 10.0), 3.0, color);
        p.text(
            Pos2::new(rect.right() - x + 8.0, rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            label,
            theme::regular(10.5),
            theme::TEXT_DIM,
        );
    };
    key(110.0, NETWORK, "Network");
    key(50.0, DISK, "Disk");
}

fn controls(ui: &mut Ui, client: &mut Client, a: &ActiveDownload) {
    let app = a.item.request.app;
    let idle = a.stopping.is_none();
    ui.spacing_mut().item_spacing.x = 8.0;
    if widgets::button(ui, "Cancel", ButtonStyle::Secondary, idle).clicked() {
        client.send(Command::CancelDownload(app));
    }
    if widgets::button(ui, "Pause", ButtonStyle::Secondary, idle).clicked() {
        client.send(Command::PauseDownload(app));
    }
}

fn stat(ui: &mut Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label).font(theme::bold(11.0)).color(theme::TEXT_DIM));
        ui.label(RichText::new(value).font(theme::medium(14.0)).color(theme::TEXT));
    });
}

/// The frame every list row shares: thumbnail, name and a one-line caption on the left, buttons on the right.
fn list_row(
    ui: &mut Ui,
    client: &Client,
    tex: &mut Textures,
    nav: &mut Option<Nav>,
    item: &DownloadItem,
    caption: &str,
    buttons: impl FnOnce(&mut Ui),
) {
    let req = &item.request;
    egui::Frame::new().fill(theme::BG_CARD).inner_margin(egui::Margin::same(8)).corner_radius(theme::RADIUS).show(
        ui,
        |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                thumb(ui, client, tex, req.app, THUMB * 0.6);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    if widgets::link(ui, &req.name).clicked() {
                        *nav = Some(Nav::Game(req.app));
                    }
                    ui.label(RichText::new(caption).color(theme::TEXT_DIM).small());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    buttons(ui);
                });
            });
        },
    );
}

fn queued_row(
    ui: &mut Ui,
    client: &mut Client,
    tex: &mut Textures,
    nav: &mut Option<Nav>,
    item: &DownloadItem,
    index: usize,
    last: usize,
) {
    let app = item.request.app;
    let caption = format!("{}  {}  waiting", item.kind.label(), item.request.library.display());
    let (mut remove, mut pause, mut up, mut down, mut first) = (false, false, false, false, false);
    list_row(ui, client, tex, nav, item, &caption, |ui| {
        remove = widgets::icon_button(ui, icon::X, 30.0, "Remove from the queue").clicked();
        pause = widgets::icon_button(ui, icon::PAUSE, 30.0, "Pause").clicked();
        if index < last {
            down = widgets::icon_button(ui, icon::CARET_DOWN, 30.0, "Move down").clicked();
        }
        if index > 0 {
            up = widgets::icon_button(ui, icon::CARET_UP, 30.0, "Move up").clicked();
            first = widgets::icon_button(ui, icon::CARET_DOUBLE_UP, 30.0, "Download next").clicked();
        }
    });
    if remove {
        client.send(Command::CancelDownload(app));
    } else if pause {
        client.send(Command::PauseDownload(app));
    } else if first {
        client.send(Command::Manage(ManageCmd::PrioritizeDownload(app)));
    } else if up {
        client.send(Command::Manage(ManageCmd::MoveDownload(app, -1)));
    } else if down {
        client.send(Command::Manage(ManageCmd::MoveDownload(app, 1)));
    }
}

fn paused_row(ui: &mut Ui, client: &mut Client, tex: &mut Textures, nav: &mut Option<Nav>, p: &PausedDownload) {
    let app = p.item.request.app;
    let done = if p.total_bytes > 0 {
        format!("{} of {}", fmt::bytes(p.done_bytes), fmt::bytes(p.total_bytes))
    } else {
        "not started".into()
    };
    let caption = format!("{}  paused, {done}", p.item.kind.label());
    let (mut resume, mut cancel) = (false, false);
    list_row(ui, client, tex, nav, &p.item, &caption, |ui| {
        cancel = widgets::button(ui, "Remove", ButtonStyle::Secondary, true).clicked();
        resume = widgets::button(ui, "Resume", ButtonStyle::Primary, true).clicked();
    });
    if resume {
        client.send(Command::ResumeDownload(app));
    }
    if cancel {
        client.send(Command::CancelDownload(app));
    }
}

fn finished_row(ui: &mut Ui, client: &Client, tex: &mut Textures, nav: &mut Option<Nav>, f: &FinishedDownload) {
    let result = match &f.outcome {
        Outcome::Done { up_to_date: true, .. } => "already up to date".to_string(),
        Outcome::Done { bytes_downloaded, seconds, .. } => {
            format!("done, {} in {}", fmt::bytes(*bytes_downloaded), fmt::duration(*seconds as u64))
        }
        Outcome::Failed(e) => format!("failed: {e}"),
        Outcome::Cancelled => "cancelled".into(),
    };
    let caption = format!("{}  {result}  {}", f.item.kind.label(), fmt::ago(f.at, fmt::now_unix()));
    list_row(ui, client, tex, nav, &f.item, &caption, |ui| {
        let (glyph, color) = match f.outcome {
            Outcome::Done { .. } => (icon::CHECK_CIRCLE, theme::VERIFY),
            Outcome::Failed(_) => (icon::WARNING_CIRCLE, theme::ERROR_TEXT),
            Outcome::Cancelled => (icon::PROHIBIT, theme::TEXT_DIM),
        };
        ui.label(RichText::new(glyph).font(theme::regular(20.0)).color(color));
    });
}
