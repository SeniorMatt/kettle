//! A game's page: hero banner with logo, the action bar (Play / Install and
//! stats), and details below.

use crate::textures::{Art, Textures};
use crate::{fmt, theme, widgets};
use egui::{Color32, Pos2, Rect, RichText, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{Client, Command};
use sr_model::{AppId, ArtKind, InstallState};

pub fn show(ui: &mut Ui, client: &mut Client, tex: &mut Textures, app: AppId) {
    let Some(game) = client.state().game(app).cloned() else {
        client.send(Command::Select(None));
        return;
    };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        hero(ui, client, tex, app, &game.name);
        action_bar(ui, client, &game);
        details(ui, &game);
    });
}

fn hero(ui: &mut Ui, client: &Client, tex: &mut Textures, app: AppId, name: &str) {
    let w = ui.available_width();
    let h = (w * theme::HERO_ASPECT).clamp(240.0, 460.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
    match tex.get(client, app, ArtKind::Hero) {
        Art::Ready(t, a) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(a)),
        Art::Loading => widgets::placeholder(ui, rect),
        Art::Missing => widgets::vgradient(ui, rect, theme::BG_CAPSULE, theme::BG_MAIN),
    }
    // Fade into the action bar.
    let fade = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - 90.0), rect.right_bottom());
    widgets::vgradient(ui, fade, Color32::TRANSPARENT, Color32::from_black_alpha(140));

    let logo_bounds =
        Rect::from_min_size(Pos2::new(rect.left() + 40.0, rect.top() + h * 0.25), Vec2::new(w * 0.36, h * 0.6));
    match tex.get(client, app, ArtKind::Logo) {
        Art::Ready(t, a) => widgets::image_fit_bottom_left(ui, logo_bounds, t, a),
        Art::Missing => {
            let p = Pos2::new(logo_bounds.left(), logo_bounds.bottom());
            ui.painter().text(p, egui::Align2::LEFT_BOTTOM, name, theme::bold(40.0), Color32::WHITE);
        }
        Art::Loading => {}
    }
}

fn action_bar(ui: &mut Ui, client: &mut Client, game: &sr_model::Game) {
    let frame = egui::Frame::new().fill(theme::BG_ACTION_BAR).inner_margin(egui::Margin::symmetric(32, 16));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 28.0;
            let resp = match game.install {
                InstallState::Installed { .. } => {
                    widgets::action_button(ui, icon::PLAY, "PLAY", (theme::PLAY_A, theme::PLAY_B))
                }
                InstallState::UpdateRequired { .. } => {
                    widgets::action_button(ui, icon::DOWNLOAD_SIMPLE, "UPDATE", (theme::INSTALL_A, theme::INSTALL_B))
                }
                _ => widgets::action_button(ui, icon::DOWNLOAD_SIMPLE, "INSTALL", (theme::INSTALL_A, theme::INSTALL_B)),
            };
            if resp.clicked() {
                let cmd = if game.install.is_installed() {
                    Command::Play(game.app_id)
                } else {
                    Command::Install(game.app_id)
                };
                client.send(cmd);
            }
            stat(ui, "LAST PLAYED", &fmt::last_played(game.last_played, fmt::now_unix()));
            stat(ui, "PLAY TIME", &fmt::playtime(game.playtime_minutes));
            if let InstallState::Installed { size_bytes, .. } | InstallState::UpdateRequired { size_bytes, .. } =
                game.install
            {
                stat(ui, "SIZE ON DISK", &fmt::bytes(size_bytes));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                if widgets::icon_button(ui, icon::SHOPPING_CART, 34.0, "Store page").clicked() {
                    client.send(Command::OpenStorePage(game.app_id));
                }
                widgets::icon_button(ui, icon::STAR, 34.0, "Add to Favorites");
                widgets::icon_button(ui, icon::INFO, 34.0, "Game info");
                widgets::icon_button(ui, icon::GEAR, 34.0, "Manage");
            });
        });
    });
}

fn stat(ui: &mut Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(label).font(theme::bold(11.0)).color(theme::TEXT_DIM));
        ui.add_space(3.0);
        ui.label(RichText::new(value).font(theme::medium(14.0)).color(theme::TEXT));
    });
}

fn details(ui: &mut Ui, game: &sr_model::Game) {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(32, 24)).show(ui, |ui| {
        ui.label(RichText::new("ACTIVITY").font(theme::bold(13.0)).color(theme::TEXT_HEADING));
        ui.add_space(8.0);
        let text = match game.install {
            InstallState::Installed { ref path, .. } | InstallState::UpdateRequired { ref path, .. } => {
                format!("Installed in {path}")
            }
            _ => "Not installed.".to_string(),
        };
        ui.label(RichText::new(text).color(theme::TEXT_DIM));
        ui.add_space(4.0);
        ui.label(RichText::new(format!("App ID {}", game.app_id)).color(theme::TEXT_DIM).small());
    });
}
