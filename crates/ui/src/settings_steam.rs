//! Settings > In Game, the Steam client rows: whether Kettle starts the official client before you press Play, and
//! the experimental mode that runs Steamworks games without it.

use crate::widgets;
use egui::Ui;
use sr_client::{Client, Command, SteamBackground};

/// The wording of each "Start Steam in the background" choice.
pub fn background_label(mode: SteamBackground) -> &'static str {
    match mode {
        SteamBackground::Automatic => "Automatic",
        SteamBackground::Always => "Always",
        SteamBackground::Never => "Never",
    }
}

/// The text of the Play button while a game waits for the Steam client.
pub fn starting_label() -> &'static str {
    "Starting Steam…"
}

/// The rows for Settings > In Game.
pub fn rows(ui: &mut Ui, client: &mut Client) {
    let mut mode = client.state().settings.steam_background;
    let before = mode;
    let desc = "Games that use the Steamworks API only start while the official Steam client runs. Automatic starts \
                it quietly (no window, low priority) when Kettle opens, but only if you have such a game installed, \
                so Play does not wait. It keeps running after Kettle closes, like Steam's tray icon. Never: Kettle \
                does not start it at all.";
    widgets::setting_row(ui, "Start Steam in the background", desc, |ui| {
        widgets::combo(ui, "steam-background", 200.0, background_label(mode), |ui| {
            for m in [SteamBackground::Automatic, SteamBackground::Always, SteamBackground::Never] {
                ui.selectable_value(&mut mode, m, background_label(m));
            }
        });
    });
    if mode != before {
        client.send(Command::SetSteamBackground(mode));
    }
    let mut own = client.state().settings.own_steamclient;
    let desc = "Experimental, off until tested on your games. Steamworks games use Kettle's own Steam library and \
                your signed-in account instead of the official client, which is then never started. Needs Kettle's \
                libsteamclient.so next to the program and no official Steam install in ~/.steam; otherwise Kettle \
                starts the official client as usual.";
    let mut changed = false;
    widgets::setting_row(ui, "Run games without the Steam client (experimental)", desc, |ui| {
        changed = widgets::toggle(ui, &mut own).changed();
    });
    if changed {
        client.send(Command::SetOwnSteamclient(own));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_distinct() {
        let l = [SteamBackground::Automatic, SteamBackground::Always, SteamBackground::Never].map(background_label);
        assert!(l[0] != l[1] && l[1] != l[2] && l[0] != l[2]);
        assert!(starting_label().starts_with("Starting Steam"));
    }
}
