//! A game's page: hero banner with logo, the action bar (Play / Install and
//! stats), and details below.

mod recommend;
mod sections;

use crate::dialogs::Overlays;
use crate::textures::{Art, Textures};
use crate::{anim, fmt, steam_icons, theme, widgets};
use egui::{Color32, Id, Pos2, Rect, RichText, Ui, Vec2};
use sr_client::{Client, CollCmd, Command, DownloadStatus};
use sr_model::{AppId, ArtKind, InstallState};

pub fn show(ui: &mut Ui, client: &mut Client, tex: &mut Textures, app: AppId, ov: &mut Overlays) {
    let Some(game) = client.state().game(app).cloned() else {
        client.send(Command::Select(None));
        return;
    };
    page_background(ui);
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        hero(ui, client, tex, &game);
        action_bar(ui, client, &game, ov);
        details(ui, client, tex, &game);
    });
}

/// Flat down to the middle of the page, then a darkening towards the bottom, as in the official client.
fn page_background(ui: &Ui) {
    let r = ui.max_rect();
    let mid = r.top() + r.height() * 0.5;
    ui.painter().rect_filled(Rect::from_min_max(r.min, Pos2::new(r.right(), mid)), 0.0, theme::PAGE_BG_TOP);
    let mut mesh = egui::epaint::Mesh::default();
    for (pos, c) in [
        (Pos2::new(r.left(), mid), theme::PAGE_BG_TOP),
        (Pos2::new(r.right(), mid), theme::PAGE_BG_TOP),
        (r.right_bottom(), theme::PAGE_BG_BOTTOM),
        (r.left_bottom(), theme::PAGE_BG_BOTTOM),
    ] {
        mesh.colored_vertex(pos, c);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(mesh);
}

fn hero(ui: &mut Ui, client: &Client, tex: &mut Textures, game: &sr_model::Game) {
    let (app, name) = (game.app_id, game.name.as_str());
    let w = ui.available_width();
    let h = (w * theme::HERO_ASPECT).clamp(180.0, 460.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), egui::Sense::hover());
    match tex.get(client, app, ArtKind::Hero) {
        Art::Ready(t, a) => widgets::image_cover(ui, rect, t, Color32::WHITE.gamma_multiply(a)),
        Art::Loading => widgets::placeholder(ui, rect),
        Art::Missing => {
            ui.painter().rect_filled(rect, 0.0, theme::BG_CAPSULE);
        }
    }
    // "INSTALLED" tag in the top right corner.
    if game.install.is_installed() {
        let g = ui.painter().layout_no_wrap("INSTALLED".to_string(), theme::regular(12.0), theme::TEXT_HEADING);
        let tag = Rect::from_min_size(
            Pos2::new(rect.right() - 20.0 - g.size().x - 12.0, rect.top() + 16.0),
            g.size() + Vec2::new(12.0, 8.0),
        );
        ui.painter().rect_filled(tag, 2.0, Color32::from_black_alpha(110));
        ui.painter().galley(tag.min + Vec2::new(6.0, 4.0), g, theme::TEXT_HEADING);
    }
    let logo_bounds = Rect::from_min_max(
        Pos2::new(rect.left() + 28.0, rect.top() + h * 0.2),
        Pos2::new(rect.left() + 28.0 + w * 0.36, rect.bottom() - 20.0),
    );
    match tex.get(client, app, ArtKind::Logo) {
        Art::Ready(t, a) => widgets::image_fit_bottom_left(ui, logo_bounds, t, a),
        Art::Missing => {
            let p = Pos2::new(logo_bounds.left(), logo_bounds.bottom());
            ui.painter().text(p, egui::Align2::LEFT_BOTTOM, name, theme::bold(36.0), Color32::WHITE);
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
    let frame = egui::Frame::new().fill(theme::BG_ACTION_BAR).inner_margin(egui::Margin {
        left: 20,
        right: 23,
        top: 10,
        bottom: 9,
    });
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            primary_button(ui, client, game, ov);
            ui.add_space(62.0);
            stat(ui, "LAST PLAYED", &fmt::last_played(game.last_played, fmt::now_unix()));
            ui.add_space(18.0);
            stat(ui, "PLAY TIME", &fmt::playtime(game.playtime_minutes));
            ui.add_space(18.0);
            // The client skips games with a Linux build; nothing is drawn for them.
            crate::protondb_badge::badge(ui, client, game.app_id, false);
            ui.add_space(18.0);
            crate::cloud_ui::status(ui, client, game.app_id);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                // Left to right on screen: gear, info, heart.
                let fav = client.state().collections.is_favorite(game.app_id);
                let heart = square_button(ui, Icon::Heart, fav, "Favorite");
                if heart.clicked() {
                    client.send(Command::Collections(CollCmd::SetFavorite(game.app_id, !fav)));
                }
                let info = square_button(ui, Icon::Info, false, "Game info");
                info_menu(&info, game);
                let gear = square_button(ui, Icon::Gear, false, "Manage");
                crate::game_menu::manage(&gear, client, game, ov);
            });
        });
    });
}

/// The icons of the page's buttons.
#[derive(Clone, Copy, PartialEq)]
enum Icon {
    Gear,
    Info,
    Heart,
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

/// Steam's action-bar icon button: a 32 px square, 4 px radius, flat grey fill, light icon.
fn square_button(ui: &mut Ui, icon: Icon, active: bool, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(theme::PAGE_ICON_BTN), egui::Sense::click());
    let t = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), 0.12);
    let bg = theme::PAGE_ICON_BTN_BG.lerp_to_gamma(theme::PAGE_ICON_BTN_BG_HOVER, t);
    ui.painter().rect_filled(rect, 4.0, bg);
    let base = if active { theme::ACCENT } else { Color32::from_rgb(0xa3, 0xa2, 0xa7) };
    let color = base.lerp_to_gamma(Color32::WHITE, 0.5 * t);
    let p = ui.painter();
    match icon {
        Icon::Gear => steam_icons::gear(p, rect.center(), 17.0, color, bg),
        Icon::Info => steam_icons::info(p, rect.center(), 17.0, color, bg),
        Icon::Heart => steam_icons::heart(p, rect.center(), 17.0, color),
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip)
}

/// What the big button shows before its label.
#[derive(Clone, Copy)]
enum Glyph {
    Play,
    Download,
    Stop,
}

/// The big button: 33 px high, the icon and the upper-case label. `colors.0` is the button, `colors.1` the
/// separate drop-down arrow segment next to it (None for buttons without one). Returns both responses.
fn big_button(
    ui: &mut Ui,
    glyph: Glyph,
    label: &str,
    colors: (Color32, Option<Color32>),
    enabled: bool,
) -> (egui::Response, Option<egui::Response>) {
    let text_color = if matches!(glyph, Glyph::Play) { Color32::from_rgb(0xdf, 0xff, 0xe5) } else { Color32::WHITE };
    let galley = ui.painter().layout_no_wrap(label.to_uppercase(), theme::regular(17.0), text_color);
    let size =
        Vec2::new((galley.size().x + 15.0 + 12.0 + 11.0 + 14.0).max(theme::PAGE_PLAY_MIN_W), theme::PAGE_ACTION_H);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let t = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered() && enabled, 0.12);
    let bg = if enabled {
        widgets::shade(colors.0, t, resp.is_pointer_button_down_on())
    } else {
        theme::PAGE_ICON_BTN_BG_HOVER
    };
    ui.painter().rect_filled(rect, 2.0, bg);
    let fg = if enabled { text_color } else { theme::TEXT_DIM };
    let icon_c = Pos2::new(rect.left() + 15.0 + 6.0, rect.center().y);
    match glyph {
        Glyph::Play => steam_icons::play_triangle(ui.painter(), icon_c, 12.0, fg),
        Glyph::Download => steam_icons::arrow_down(ui.painter(), icon_c, 12.0, fg),
        Glyph::Stop => steam_icons::window_close(ui.painter(), icon_c, fg),
    }
    ui.painter().galley_with_override_text_color(
        Pos2::new(rect.left() + 15.0 + 12.0 + 11.0, rect.center().y - galley.size().y / 2.0),
        galley,
        fg,
    );
    let main = if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
    let arrow = colors.1.map(|c| {
        ui.add_space(3.0);
        let (r, resp) = ui.allocate_exact_size(Vec2::new(24.0, theme::PAGE_ACTION_H), egui::Sense::click());
        let t = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), 0.12);
        ui.painter().rect_filled(r, 2.0, widgets::shade(c, t, resp.is_pointer_button_down_on()));
        steam_icons::play_dropdown(ui.painter(), r.center(), Color32::WHITE);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    });
    (main, arrow)
}

/// Play, Stop, Install, Update or the state of the download, in the same spot.
fn primary_button(ui: &mut Ui, client: &mut Client, game: &sr_model::Game, ov: &mut Overlays) {
    let app = game.app_id;
    let play = (theme::PAGE_PLAY_A, Some(theme::PAGE_PLAY_B));
    let blue = (theme::PAGE_STOP_A, None);
    if client.state().running.contains_key(&app) {
        if big_button(ui, Glyph::Stop, "Stop", blue, true).0.clicked() {
            client.send(Command::Stop(app));
        }
        return;
    }
    if client.state().starting_steam.contains(&app) {
        big_button(ui, Glyph::Play, crate::settings_steam::starting_label(), (theme::PAGE_PLAY_A, None), false);
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
        if big_button(ui, Glyph::Download, &label, blue, true).0.clicked() {
            ov.goto = Some(crate::Nav::Downloads);
        }
        return;
    }
    match game.install {
        InstallState::Installed { .. } => {
            let (main, arrow) = big_button(ui, Glyph::Play, "Play", play, true);
            if main.clicked() {
                client.send(Command::Play(app));
            }
            if let Some(arrow) = arrow {
                crate::game_menu::play_menu(&arrow, client, game, ov);
            }
        }
        InstallState::UpdateRequired { .. } => {
            if big_button(ui, Glyph::Download, "Update", blue, true).0.clicked() {
                client.send(Command::Update(app));
            }
        }
        _ => {
            if big_button(ui, Glyph::Download, "Install", blue, true).0.clicked() {
                ov.open_install(app, &game.name);
            }
        }
    }
}

fn stat(ui: &mut Ui, label: &str, value: &str) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(label).font(theme::regular(theme::PAGE_STAT_LABEL)).color(theme::PAGE_STAT_LABEL_COLOR));
        ui.label(RichText::new(value).font(theme::regular(theme::PAGE_STAT_VALUE)).color(theme::PAGE_STAT_VALUE_COLOR));
    });
}

/// The box under the play bar: Store Page, Community Hub and Discussions, and "..." for the rest. Store Page
/// opens inside Kettle, the others in the browser.
fn tab_strip(ui: &mut Ui, client: &mut Client, game: &sr_model::Game) {
    let id = game.app_id.0;
    let name = game.name.replace(' ', "%20");
    let tabs: [(&str, Option<String>); 7] = [
        ("Store Page", None),
        ("Community Hub", Some(format!("https://steamcommunity.com/app/{id}"))),
        ("Discussions", Some(format!("https://steamcommunity.com/app/{id}/discussions/"))),
        ("DLC", Some(format!("https://store.steampowered.com/dlc/{id}"))),
        ("Find Groups", Some(format!("https://steamcommunity.com/search/groups/?text={name}"))),
        ("Guides", Some(format!("https://steamcommunity.com/app/{id}/guides/"))),
        ("Support", Some(format!("https://help.steampowered.com/en/wizard/HelpWithGame/?appid={id}"))),
    ];
    let mut open: Option<(usize, Option<String>)> = None;
    let w = ui.available_width() - 40.0;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        let (rect, _) = ui.allocate_exact_size(Vec2::new(w, theme::PAGE_TAB_H), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, theme::TAB_BOX_BG);
        ui.painter().rect_stroke(rect, 2.0, egui::Stroke::new(1.0, theme::TAB_BOX_BORDER), egui::StrokeKind::Inside);
        let mut x = rect.left() + 23.0;
        for (i, (label, url)) in tabs.iter().take(3).enumerate() {
            let g =
                ui.painter().layout_no_wrap(label.to_string(), theme::regular(theme::PAGE_TAB_FONT), theme::TAB_TEXT);
            let r = Rect::from_min_size(Pos2::new(x, rect.center().y - g.size().y / 2.0), g.size());
            let resp = ui.interact(r.expand2(Vec2::new(12.0, 10.0)), Id::new(("game-tab", i)), egui::Sense::click());
            let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
            ui.painter().galley_with_override_text_color(r.min, g, theme::TAB_TEXT.lerp_to_gamma(theme::TEXT, t));
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                open = Some((i, url.clone()));
            }
            x += r.width() + 68.0;
        }
        // "..." with the rest of the links.
        let c = Pos2::new(rect.right() - 44.0, rect.center().y);
        let more = ui.interact(
            Rect::from_center_size(c, Vec2::new(36.0, 28.0)),
            Id::new("game-tab-more"),
            egui::Sense::click(),
        );
        let t = anim::animate_bool(ui.ctx(), more.id.with("h"), more.hovered(), anim::FAST);
        steam_icons::ellipsis(ui.painter(), c, theme::TAB_TEXT.lerp_to_gamma(theme::TEXT, t));
        widgets::popup_menu(&more.on_hover_cursor(egui::CursorIcon::PointingHand), |ui| {
            for (i, (label, url)) in tabs.iter().enumerate().skip(3) {
                if ui.button(*label).clicked() {
                    open = Some((i, url.clone()));
                    ui.close();
                }
            }
        });
    });
    if let Some((_, url)) = open {
        match url {
            Some(u) => client.send(Command::OpenUrl(u)),
            None => client.send(Command::OpenStorePage(game.app_id)),
        }
    }
}

fn details(ui: &mut Ui, client: &mut Client, tex: &mut Textures, game: &sr_model::Game) {
    tab_strip(ui, client, game);
    egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 23, top: 24, bottom: 24 }).show(ui, |ui| {
        recommend::show(ui, client, game);
        sections::show(ui, client, tex, game);
    });
}
