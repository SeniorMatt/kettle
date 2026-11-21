//! "Add a Game": the dialog for non-Steam games, and the small helpers a game page or menu needs.
//!
//! Hooks:
//! - keep an `Option<AddGameDialog>` next to the other dialogs; set it to `Some(AddGameDialog::default())` from
//!   the "Add a Game" menu item and call `dialog.show(ctx, client)` every frame while it is `Some`
//!   (it returns `false` when it should be closed);
//! - on a game's page or context menu: [`is_non_steam`] to show the "Non-Steam" label and hide Steam-only
//!   actions (install, verify, store page), and [`remove_item`] for "Remove from library".

use crate::theme;
use crate::widgets::{self, ButtonStyle};
use egui::{Context, Id, RichText, Ui};
use sr_client::{Client, Command, NonSteamCmd, installed_desktop_apps, read_desktop_file};
use sr_model::AppId;
use std::path::PathBuf;

const DIALOG_W: f32 = 520.0;

/// Is this a game added by hand or imported from Steam's shortcuts?
pub fn is_non_steam(client: &Client, app: AppId) -> bool {
    client.state().non_steam.contains(app)
}

/// "Remove from library" for a non-Steam game (the program itself stays on disk). Returns true when clicked.
pub fn remove_item(ui: &mut Ui, client: &mut Client, app: AppId) -> bool {
    if !is_non_steam(client, app) {
        return false;
    }
    let clicked = widgets::button(ui, "Remove from library", ButtonStyle::Secondary, true).clicked();
    if clicked {
        client.send(Command::NonSteam(NonSteamCmd::Remove(app)));
    }
    clicked
}

#[derive(Default)]
pub struct AddGameDialog {
    name: String,
    exe: String,
    args: String,
    icon: Option<PathBuf>,
    /// Installed apps (name, `.desktop` file), read once when the dialog first shows.
    apps: Option<Vec<(String, PathBuf)>>,
    error: Option<String>,
}

impl AddGameDialog {
    /// Show the dialog. Returns false once it was closed (added, imported or cancelled).
    pub fn show(&mut self, ctx: &Context, client: &mut Client) -> bool {
        let apps = self.apps.get_or_insert_with(installed_desktop_apps).clone();
        let mut keep = true;
        let mut add = false;
        let mut import = false;
        let frame = egui::Frame::new()
            .fill(theme::MODAL_BG)
            .inner_margin(egui::Margin::same(theme::MODAL_PAD))
            .corner_radius(theme::BUTTON_RADIUS)
            .stroke(egui::Stroke::new(1.0, theme::WINDOW_BORDER));
        let modal = egui::Modal::new(Id::new("add-game-dialog")).frame(frame).show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            theme::dialog_style(ui);
            widgets::page_title(ui, "Add a Game");
            ui.label(
                RichText::new(
                    "Add any program to your library. It starts directly; Windows programs (.exe) start with Proton.",
                )
                .font(theme::regular(13.0))
                .color(theme::DIALOG_DESC),
            );
            widgets::field_label(ui, "From an installed app");
            let mut picked: Option<PathBuf> = None;
            widgets::combo(ui, "add-game-apps", DIALOG_W, "Choose an app...", |ui| {
                for (name, file) in &apps {
                    if ui.selectable_label(false, name).clicked() {
                        picked = Some(file.clone());
                    }
                }
            });
            if let Some(file) = picked {
                match read_desktop_file(&file) {
                    Ok(e) => {
                        self.name = e.name;
                        self.exe = e.exe.display().to_string();
                        self.args = e.args;
                        self.icon = e.icon;
                        self.error = None;
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            widgets::field_label(ui, "Program (full path)");
            widgets::text_input(ui, &mut self.exe, DIALOG_W, "/home/me/games/game.sh or /home/me/games/Game.exe");
            widgets::field_label(ui, "Name");
            widgets::text_input(ui, &mut self.name, DIALOG_W, "Taken from the file name when empty");
            widgets::field_label(ui, "Arguments (optional)");
            widgets::text_input(ui, &mut self.args, DIALOG_W, "-fullscreen");
            if let Some(e) = &self.error {
                ui.add_space(8.0);
                ui.label(RichText::new(e).font(theme::regular(13.0)).color(theme::ERROR_TEXT));
            }
            ui.add_space(14.0);
            widgets::button_row(ui, |ui| {
                let ready = self.exe.trim().starts_with('/');
                add = widgets::button(ui, "Add game", ButtonStyle::Primary, ready).clicked();
                if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                    keep = false;
                }
                import = widgets::button(ui, "Import from Steam", ButtonStyle::Secondary, true)
                    .on_hover_text(
                        "Reads the non-Steam shortcuts of the official client. Steam's files are not changed.",
                    )
                    .clicked();
            });
        });
        if modal.should_close() {
            keep = false;
        }
        if add {
            client.send(Command::NonSteam(NonSteamCmd::Add {
                name: self.name.clone(),
                exe: PathBuf::from(self.exe.trim()),
                args: self.args.clone(),
                icon: self.icon.clone(),
            }));
            keep = false;
        }
        if import {
            client.send(Command::NonSteam(NonSteamCmd::ImportSteam));
            keep = false;
        }
        keep
    }
}
