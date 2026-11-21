//! Extra settings rows: "Start with the computer" and the Kettle update rows.
//!
//! Hook: call [`general_rows`] from the General (or Interface) settings page, next to "Start minimized".

use crate::widgets;
use egui::Ui;
use sr_client::{Client, Command};

/// "Start with the computer" (writes `~/.config/autostart/kettle.desktop`).
pub fn start_with_computer_row(ui: &mut Ui, client: &mut Client) {
    let mut on = client.state().autostart;
    let minimized = client.state().settings.interface.start_minimized;
    let desc = if minimized {
        "Kettle starts when you log in, with its window hidden in the tray (\"Start minimized\" is on)."
    } else {
        "Kettle starts when you log in. Turn on \"Start minimized\" to keep the window hidden."
    };
    widgets::setting_row(ui, "Start with the computer", desc, |ui| {
        if widgets::toggle(ui, &mut on).changed() {
            client.send(Command::SetAutostart(on));
        }
    });
}

/// The rows of this file in one go: autostart, then the update rows.
pub fn general_rows(ui: &mut Ui, client: &mut Client) {
    start_with_computer_row(ui, client);
    crate::kettle_update_ui::settings_rows(ui, client);
}
