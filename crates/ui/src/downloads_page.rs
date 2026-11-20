//! The Downloads page: the active download with a gliding progress bar, speed, ETA and controls, then the
//! queue, paused downloads and the recently finished list.

use crate::textures::{Art, Textures};
use crate::widgets::{self, ButtonStyle};
use crate::{Nav, anim, fmt, theme};
use egui::{Color32, Id, RichText, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{
    ActiveDownload, Client, Command, DownloadItem, DownloadPhase, FinishedDownload, Outcome, PausedDownload, StopKind,
};
use sr_model::{AppId, ArtKind};

/// How fast the bar glides toward the newest number (per second).
const GLIDE: f32 = 4.0;
const THUMB: Vec2 = Vec2::new(150.0, 70.0);

pub fn show(ui: &mut Ui, client: &mut Client, tex: &mut Textures, nav: &mut Option<Nav>) {
    let state = client.state().downloads.clone();
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
            if let Some(a) = &state.active {
                active_card(ui, client, tex, a, nav);
            }
            if !state.queue.is_empty() {
                ui.add_space(14.0);
                widgets::section_title(ui, &format!("UP NEXT ({})", state.queue.len()));
                for item in &state.queue {
                    queued_row(ui, client, tex, nav, item);
                }
            }
            if !state.paused.is_empty() {
                ui.add_space(14.0);
                widgets::section_title(ui, &format!("PAUSED ({})", state.paused.len()));
                for p in &state.paused {
                    paused_row(ui, client, tex, nav, p);
                }
            }
            if !state.finished.is_empty() {
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    widgets::section_title(ui, "RECENTLY FINISHED");
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

fn thumb(ui: &mut Ui, client: &Client, tex: &mut Textures, app: AppId) {
    let (rect, _) = ui.allocate_exact_size(THUMB, Sense::hover());
    match tex.get(client, app, ArtKind::Header) {
        Art::Ready(t, a) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(a)),
        Art::Loading => widgets::placeholder(ui, rect),
        Art::Missing => {
            ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
        }
    }
}

fn phase_text(a: &ActiveDownload) -> String {
    if let Some(s) = a.stopping {
        return if s == StopKind::Pause { "Pausing...".into() } else { "Cancelling...".into() };
    }
    let done_of = || format!("{} of {}", fmt::bytes(a.done_bytes), fmt::bytes(a.total_bytes));
    match a.phase {
        DownloadPhase::Preparing => "Preparing: fetching the game's file list".into(),
        DownloadPhase::Downloading => format!("Downloading, {}", done_of()),
        DownloadPhase::Verifying => format!("Verifying files, {}", done_of()),
        DownloadPhase::Finished => "Finishing".into(),
        DownloadPhase::Failed => "Failed".into(),
    }
}

fn active_card(ui: &mut Ui, client: &mut Client, tex: &mut Textures, a: &ActiveDownload, nav: &mut Option<Nav>) {
    let req = &a.item.request;
    let frame =
        egui::Frame::new().fill(theme::BG_CARD).inner_margin(egui::Margin::same(14)).corner_radius(theme::RADIUS);
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            thumb(ui, client, tex, req.app);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                if widgets::link(ui, &req.name).clicked() {
                    *nav = Some(Nav::Game(req.app));
                }
                ui.label(
                    RichText::new(format!("{}  {}", a.item.kind.label(), req.library.display()))
                        .color(theme::TEXT_DIM)
                        .small(),
                );
                ui.label(RichText::new(phase_text(a)).color(theme::TEXT));
                let shown = anim::glide(ui.ctx(), Id::new(("dl-bar", req.app.0)), a.fraction(), GLIDE);
                let fill = if a.phase == DownloadPhase::Verifying { theme::VERIFY } else { theme::ACCENT };
                widgets::progress_bar(ui, (ui.available_width() - 4.0).max(100.0), 8.0, shown, fill);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    stat(ui, "SPEED", &fmt::speed(a.bytes_per_sec));
                    stat(ui, "TIME LEFT", &a.eta_secs().map_or("-".into(), fmt::duration));
                    stat(ui, "PROGRESS", &format!("{:.0}%", shown * 100.0));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        controls(ui, client, a);
                    });
                });
                if let Some(m) = &a.message {
                    ui.label(RichText::new(m).color(theme::TEXT_DIM).small());
                }
            });
        });
    });
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
                let (small, _) = ui.allocate_exact_size(THUMB * 0.6, Sense::hover());
                match tex.get(client, req.app, ArtKind::Header) {
                    Art::Ready(t, a) => widgets::image_cover(ui, small, t, Color32::WHITE.gamma_multiply(a)),
                    _ => {
                        ui.painter().rect_filled(small, theme::RADIUS, theme::BG_CAPSULE);
                    }
                }
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

fn queued_row(ui: &mut Ui, client: &mut Client, tex: &mut Textures, nav: &mut Option<Nav>, item: &DownloadItem) {
    let app = item.request.app;
    let caption = format!("{}  {}  waiting", item.kind.label(), item.request.library.display());
    let mut cancel = false;
    list_row(ui, client, tex, nav, item, &caption, |ui| {
        cancel = widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked();
    });
    if cancel {
        client.send(Command::CancelDownload(app));
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
        cancel = widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked();
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
