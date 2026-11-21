//! Menus about one game: the right-click menu on covers and list rows, and the gear ("Manage") menu on the
//! game page. Only items that are built are shown; items that do not apply right now are dimmed.

use crate::dialogs::Overlays;
use crate::{theme, widgets};
use egui::{Response, Ui};
use sr_client::{Client, Command, DownloadStatus};
use sr_model::{AppId, Game, InstallState};

/// What may be done with a game right now.
struct Can {
    installed: bool,
    /// Not downloading and not running: safe to verify or remove.
    idle: bool,
    running: bool,
}

fn can(client: &Client, game: &Game) -> Can {
    let app = game.app_id;
    let running = client.state().running.contains_key(&app);
    Can {
        installed: game.install.is_installed(),
        idle: client.state().downloads.status(app) == DownloadStatus::None && !running,
        running,
    }
}

/// A menu row. Returns true when it was clicked.
fn item(ui: &mut Ui, label: &str, enabled: bool) -> bool {
    ui.add_enabled(enabled, egui::Button::new(label)).clicked()
}

/// The right-click menu of a cover or list row. Call it every frame for the row's response; it draws only
/// while open. The game is looked up only then, so closed rows cost nothing.
pub fn context(target: &Response, client: &mut Client, app: AppId, ov: &mut Overlays) {
    widgets::context_menu(target, |ui| {
        let Some(game) = client.state().game(app).cloned() else { return };
        let c = can(client, &game);
        primary(ui, client, &game, &c, ov);
        widgets::menu_separator(ui);
        // Add to Favorites, Add to >, Manage >: as in the official client's menu.
        crate::collections_ui::menu_items(ui, client, &game);
        // Inside a menu this is a submenu button (arrow on the right) that inherits the menu style.
        ui.menu_button("Manage", |ui| {
            ui.set_min_width(theme::MENU_MIN_W);
            manage_items(ui, client, &game, &c, ov)
        });
        widgets::menu_separator(ui);
        if item(ui, "Properties...", true) {
            ov.open_properties(app);
        }
    });
}

/// The gear menu on the game page.
pub fn manage(anchor: &Response, client: &mut Client, game: &Game, ov: &mut Overlays) {
    let c = can(client, game);
    widgets::popup_menu(anchor, |ui| {
        manage_items(ui, client, game, &c, ov);
        widgets::menu_separator(ui);
        if item(ui, "Launch via Steam client", true) {
            client.send(Command::LaunchViaSteam(game.app_id));
        }
        widgets::menu_separator(ui);
        if item(ui, "Properties...", true) {
            ov.open_properties(game.app_id);
        }
    });
}

/// The drop-down arrow next to PLAY: the other ways to start the game, and its properties.
pub fn play_menu(anchor: &Response, client: &mut Client, game: &Game, ov: &mut Overlays) {
    widgets::popup_menu(anchor, |ui| {
        if item(ui, "Launch via Steam client", game.install.is_installed()) {
            client.send(Command::LaunchViaSteam(game.app_id));
        }
        if item(ui, "Properties...", true) {
            ov.open_properties(game.app_id);
        }
    });
}

/// Play, Update, Stop or Install, whichever fits the game's state.
fn primary(ui: &mut Ui, client: &mut Client, game: &Game, c: &Can, ov: &mut Overlays) {
    let app = game.app_id;
    if c.running {
        if item(ui, "Stop", true) {
            client.send(Command::Stop(app));
        }
        return;
    }
    let busy = client.state().downloads.status(app) != DownloadStatus::None;
    match game.install {
        InstallState::Installed { .. } => {
            if item(ui, "Play", !busy) {
                client.send(Command::Play(app));
            }
        }
        InstallState::UpdateRequired { .. } => {
            if item(ui, "Update", !busy) {
                client.send(Command::Update(app));
            }
        }
        _ => {
            if item(ui, "Install", !busy) {
                ov.open_install(app, &game.name);
            }
        }
    }
}

/// The Manage submenu: Browse local files, Hide this game, Uninstall.
fn manage_items(ui: &mut Ui, client: &mut Client, game: &Game, c: &Can, ov: &mut Overlays) {
    let app = game.app_id;
    if item(ui, "Browse local files", c.installed) {
        client.send(Command::BrowseFiles(app));
    }
    crate::collections_ui::hide_item(ui, client, game);
    widgets::menu_separator(ui);
    if crate::add_game_ui::is_non_steam(client, app) {
        // As in the official client: the program stays on disk, only the library entry goes.
        if item(ui, "Remove non-Steam game from your library", c.idle) {
            crate::add_game_ui::remove(client, app);
        }
        return;
    }
    if item(ui, "Uninstall...", c.installed && c.idle) {
        ov.open_uninstall(app);
    }
}
