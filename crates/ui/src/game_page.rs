//! A game's page: hero banner with logo, the action bar (Play / Install and
//! stats), and details below.

use crate::dialogs::Overlays;
use crate::textures::{Art, Textures};
use crate::{fmt, theme, widgets};
use egui::{Color32, Pos2, Rect, RichText, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{Client, CollCmd, Command, DownloadStatus};
use sr_model::{AppId, ArtKind, InstallState};

pub fn show(ui: &mut Ui, client: &mut Client, tex: &mut Textures, app: AppId, ov: &mut Overlays) {
    let Some(game) = client.state().game(app).cloned() else {
        client.send(Command::Select(None));
        return;
    };
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        hero(ui, client, tex, app, &game.name);
        action_bar(ui, client, &game, ov);
        details(ui, client, &game);
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

/// What a double click or Enter on a game does, like the official client: start it when it is installed
/// (finish its update first when it needs one), open the install dialog when it is not. Does nothing while
/// the game runs or downloads.
pub fn activate(client: &mut Client, ov: &mut Overlays, app: AppId) {
    let Some(game) = client.state().game(app).cloned() else { return };
    if client.state().running.contains_key(&app)
        || client.state().starting_steam.contains(&app)
        || client.state().downloads.status(app) != DownloadStatus::None
    {
        return;
    }
    match game.install {
        InstallState::Installed { .. } => client.send(Command::Play(app)),
        InstallState::UpdateRequired { .. } => client.send(Command::Update(app)),
        _ => ov.open_install(app, &game.name),
    }
}

fn action_bar(ui: &mut Ui, client: &mut Client, game: &sr_model::Game, ov: &mut Overlays) {
    let frame = egui::Frame::new().fill(theme::BG_ACTION_BAR).inner_margin(egui::Margin::symmetric(32, 16));
    frame.show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 32.0;
            primary_button(ui, client, game, ov);
            stat(ui, "LAST PLAYED", &fmt::last_played(game.last_played, fmt::now_unix()));
            stat(ui, "PLAY TIME", &fmt::playtime(game.playtime_minutes));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                // Left to right on screen: gear, info, favourite.
                let fav = client.state().collections.is_favorite(game.app_id);
                let star = square_button(ui, icon::STAR, if fav { theme::ACCENT } else { theme::TEXT }, "Favorite");
                if star.clicked() {
                    client.send(Command::Collections(CollCmd::SetFavorite(game.app_id, !fav)));
                }
                let info = square_button(ui, icon::INFO, theme::TEXT, "Game info");
                info_menu(&info, game);
                let gear = square_button(ui, icon::GEAR, theme::TEXT, "Manage");
                crate::game_menu::manage(&gear, client, game, ov);
            });
        });
    });
}

/// The info (i) button's popup: where the game lives and how big it is.
fn info_menu(anchor: &egui::Response, game: &sr_model::Game) {
    widgets::popup_menu(anchor, |ui| {
        widgets::menu_label(ui, &format!("App ID {}", game.app_id), true);
        if let InstallState::Installed { size_bytes, ref path, .. }
        | InstallState::UpdateRequired { size_bytes, ref path, .. } = game.install
        {
            widgets::menu_label(ui, &format!("Size on disk: {}", fmt::bytes(size_bytes)), false);
            widgets::menu_label(ui, &format!("Installed in {path}"), true);
        } else {
            widgets::menu_label(ui, "Not installed", false);
        }
    });
}

/// Steam's action-bar icon button: a 48 px square, light fill, icon centered.
fn square_button(ui: &mut Ui, glyph: &str, color: Color32, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(theme::PAGE_ICON_BTN), egui::Sense::click());
    let bg = if resp.hovered() { theme::PAGE_ICON_BTN_BG_HOVER } else { theme::PAGE_ICON_BTN_BG };
    ui.painter().rect_filled(rect, theme::RADIUS, bg);
    let c = if resp.hovered() && color == theme::TEXT { Color32::WHITE } else { color };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, theme::regular(22.0), c);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// The big button: 48 px high, at least 164 px wide, icon and bold label centered.
fn big_button(ui: &mut Ui, glyph: &str, label: &str, colors: (Color32, Color32), enabled: bool) -> egui::Response {
    let text = format!("{glyph}  {label}");
    let galley = ui.painter().layout_no_wrap(text, theme::bold(17.0), Color32::WHITE);
    let size = Vec2::new((galley.size().x + 48.0).max(theme::PAGE_PLAY_MIN_W), theme::PAGE_ACTION_H);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let k = if !enabled {
        1.0
    } else if resp.is_pointer_button_down_on() {
        0.85
    } else if resp.hovered() {
        1.15
    } else {
        1.0
    };
    if enabled {
        widgets::gradient(ui, rect, colors.0.gamma_multiply(k), colors.1.gamma_multiply(k));
    } else {
        ui.painter().rect_filled(rect, theme::RADIUS, theme::PAGE_ICON_BTN_BG_HOVER);
    }
    let fg = if enabled { Color32::WHITE } else { theme::TEXT_DIM };
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, fg);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// Play, Stop, Install, Update or the state of the download, in the same spot.
fn primary_button(ui: &mut Ui, client: &mut Client, game: &sr_model::Game, ov: &mut Overlays) {
    let app = game.app_id;
    let play = (theme::PAGE_PLAY_A, theme::PAGE_PLAY_B);
    let blue = (theme::PAGE_STOP_A, theme::PAGE_STOP_B);
    if client.state().running.contains_key(&app) {
        if big_button(ui, icon::X, "Stop", blue, true).clicked() {
            client.send(Command::Stop(app));
        }
        return;
    }
    if client.state().starting_steam.contains(&app) {
        big_button(ui, icon::PLAY, "Starting Steam...", play, false);
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        return;
    }
    let updating = matches!(game.install, InstallState::Installed { .. } | InstallState::UpdateRequired { .. });
    let label = match client.state().downloads.status(app) {
        DownloadStatus::Active(f) => {
            Some(format!("{} {:.0}%", if updating { "Updating" } else { "Installing" }, f * 100.0))
        }
        DownloadStatus::Queued => Some(if updating { "Update queued" } else { "Install queued" }.to_string()),
        DownloadStatus::Paused => Some("Paused".to_string()),
        DownloadStatus::None => None,
    };
    if let Some(label) = label {
        if big_button(ui, icon::DOWNLOAD_SIMPLE, &label, blue, true).clicked() {
            ov.goto = Some(crate::Nav::Downloads);
        }
        return;
    }
    match game.install {
        InstallState::Installed { .. } => {
            if big_button(ui, icon::PLAY, "Play", play, true).clicked() {
                client.send(Command::Play(app));
            }
        }
        InstallState::UpdateRequired { .. } => {
            if big_button(ui, icon::DOWNLOAD_SIMPLE, "Update", blue, true).clicked() {
                client.send(Command::Update(app));
            }
        }
        _ => {
            if big_button(ui, icon::DOWNLOAD_SIMPLE, "Install", blue, true).clicked() {
                ov.open_install(app, &game.name);
            }
        }
    }
}

fn stat(ui: &mut Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.add_space(4.0);
        ui.label(RichText::new(label).font(theme::regular(theme::PAGE_STAT_LABEL)).color(theme::PAGE_STAT_LABEL_COLOR));
        ui.add_space(2.0);
        ui.label(RichText::new(value).font(theme::regular(theme::PAGE_STAT_VALUE)).color(theme::TEXT));
    });
}

/// Links under the action bar. Store Page opens inside Kettle, the rest in the browser.
fn tab_strip(ui: &mut Ui, client: &mut Client, game: &sr_model::Game) {
    let id = game.app_id.0;
    let name = game.name.replace(' ', "%20");
    let tabs: [(&str, Option<String>); 7] = [
        ("Store Page", None),
        ("DLC", Some(format!("https://store.steampowered.com/dlc/{id}"))),
        ("Community Hub", Some(format!("https://steamcommunity.com/app/{id}"))),
        ("Find Groups", Some(format!("https://steamcommunity.com/search/groups/?text={name}"))),
        ("Discussions", Some(format!("https://steamcommunity.com/app/{id}/discussions/"))),
        ("Guides", Some(format!("https://steamcommunity.com/app/{id}/guides/"))),
        ("Support", Some(format!("https://help.steampowered.com/en/wizard/HelpWithGame/?appid={id}"))),
    ];
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), theme::PAGE_TAB_H), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::BG_TOP);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(32.0, 0.0))), |ui| {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 28.0;
            for (label, url) in tabs {
                if widgets::text_tab(ui, label, theme::medium(theme::PAGE_TAB_FONT), false).clicked() {
                    match url {
                        Some(u) => client.send(Command::OpenUrl(u)),
                        None => client.send(Command::OpenStorePage(game.app_id)),
                    }
                }
            }
        });
    });
}

fn details(ui: &mut Ui, client: &mut Client, game: &sr_model::Game) {
    tab_strip(ui, client, game);
    let playing: Vec<(String, bool)> = client
        .state()
        .friends
        .friends
        .iter()
        .filter(|f| f.game.as_ref().is_some_and(|g| g.app_id == game.app_id.0))
        .map(|f| (f.display_name(), true))
        .collect();
    egui::Frame::new().inner_margin(egui::Margin::symmetric(32, 24)).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            let right_w = if playing.is_empty() { 0.0 } else { 260.0 };
            ui.vertical(|ui| {
                ui.set_width((ui.available_width() - right_w - 32.0).max(200.0));
                ui.label(RichText::new("GAME INFO").font(theme::bold(13.0)).color(theme::TEXT_HEADING));
                ui.add_space(8.0);
                let text = match game.install {
                    InstallState::Installed { ref path, .. } | InstallState::UpdateRequired { ref path, .. } => {
                        format!("Installed in {path}")
                    }
                    _ => "Not installed.".to_string(),
                };
                ui.label(RichText::new(text).color(theme::TEXT_DIM));
            });
            if !playing.is_empty() {
                ui.add_space(32.0);
                ui.vertical(|ui| {
                    ui.set_width(right_w);
                    ui.label(RichText::new("FRIENDS WHO PLAY").font(theme::bold(13.0)).color(theme::TEXT_HEADING));
                    ui.add_space(8.0);
                    for (name, _) in playing.iter().take(12) {
                        ui.label(RichText::new(name).color(theme::FRIEND_INGAME));
                    }
                });
            }
        });
    });
}
