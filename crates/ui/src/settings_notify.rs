//! Settings > Notifications (desktop notifications) and the account page (sign-in, local data).

use crate::theme;
use crate::widgets::{self, ButtonStyle};
use egui::{Color32, Id, RichText, Ui};
use sr_client::{Client, Command, ManageCmd, NotifySettings};

fn toggle_row(ui: &mut Ui, label: &str, desc: &str, value: &mut bool) {
    widgets::setting_row(ui, label, desc, |ui| {
        widgets::toggle(ui, value);
    });
}

pub fn notifications(ui: &mut Ui, client: &mut Client) {
    let mut n: NotifySettings = client.state().settings.notifications.clone();
    let before = n.clone();
    let desc = "Small pop-ups from your desktop's notification service. They are made on this computer; nothing is \
                sent anywhere. On systems without a notification service nothing is shown.";
    ui.label(RichText::new(desc).font(theme::regular(12.5)).color(theme::TEXT_DIM));
    ui.add_space(6.0);
    toggle_row(ui, "Show desktop notifications", "The master switch for everything below.", &mut n.enabled);
    ui.add_enabled_ui(n.enabled, |ui| {
        toggle_row(ui, "A download finishes", "A game was installed or updated.", &mut n.download_done);
        toggle_row(ui, "A friend comes online", "", &mut n.friend_online);
        toggle_row(ui, "I get a chat message", "", &mut n.chat_message);
        toggle_row(ui, "A friend joins a game", "", &mut n.friend_game);
        toggle_row(
            ui,
            "Play a sound",
            "Asks the notification service for its own sound from your sound theme. Kettle ships no sounds.",
            &mut n.sound,
        );
    });
    widgets::setting_row(ui, "Try it", "Shows one notification now.", |ui| {
        if widgets::button(ui, "Send a test", ButtonStyle::Secondary, true).clicked() {
            client.send(Command::Manage(ManageCmd::TestNotification));
        }
    });
    if n != before {
        client.send(Command::Manage(ManageCmd::SetNotifications(n)));
    }
}

pub fn account(ui: &mut Ui, client: &mut Client) {
    let confirm_key = Id::new("account-delete-confirm");
    let mut confirming: bool = ui.data(|d| d.get_temp(confirm_key)).unwrap_or(false);
    if let Some(a) = &client.state().account {
        widgets::setting_row(ui, "Signed in as", "Your Steam account name.", |ui| {
            ui.label(RichText::new(&a.account_name).font(theme::medium(14.0)).color(Color32::WHITE));
        });
    }
    let mut keep = client.state().settings.keep_signed_in;
    let desc = "Keeps the sign-in token in your system keyring, so the next start needs no QR code. Off: Kettle forgets \
                the token now and you scan a code at the next start. The token never leaves the keyring and memory.";
    widgets::setting_row(ui, "Keep me signed in on this computer", desc, |ui| {
        if widgets::toggle(ui, &mut keep).changed() {
            client.send(Command::Manage(ManageCmd::SetKeepSignedIn(keep)));
        }
    });
    let desc = "Deletes the token from the keyring and removes the cached library of this account. To end the \
                session on Steam's side too, remove Kettle under authorized devices in the Steam Mobile App.";
    widgets::setting_row(ui, "Sign out", desc, |ui| {
        if widgets::button(ui, "Sign out", ButtonStyle::Danger, true).clicked() {
            client.send(Command::Logout);
        }
    });
    let desc = "Deletes Kettle's cache folder: cover pictures, the cached game list and release lists. Games, saves, \
                settings and your sign-in are not touched; the pictures download again when needed.";
    widgets::setting_row(ui, "Delete local data", desc, |ui| {
        if confirming {
            if widgets::button(ui, "Keep", ButtonStyle::Secondary, true).clicked() {
                confirming = false;
            }
            if widgets::button(ui, "Delete", ButtonStyle::Danger, true).clicked() {
                client.send(Command::Manage(ManageCmd::ClearCache));
                confirming = false;
            }
        } else if widgets::button(ui, "Delete local data", ButtonStyle::Secondary, true).clicked() {
            confirming = true;
        }
    });
    ui.data_mut(|d| d.insert_temp(confirm_key, confirming));
}
