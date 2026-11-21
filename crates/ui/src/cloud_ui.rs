//! Steam Cloud saves: the conflict dialog, the settings switch, the per-game switch and a small status line.
//!
//! Hooks (the pages call these; nothing here is wired in yet):
//! * [`conflict_dialog`]: once per frame next to the other dialogs;
//! * [`settings_row`]: in a settings section ("Steam Cloud");
//! * [`game_row`]: in a game's properties;
//! * [`status`]: on the game page near the Play button (shows "Syncing...", or the last result).
#![allow(dead_code)] // until the pages call these

use crate::widgets::{self, ButtonStyle};
use crate::{fmt, theme};
use egui::{Context, Id, RichText, Ui};
use sr_client::{Client, CloudChoice, CloudPhase, Command};
use sr_model::AppId;

const DIALOG_W: f32 = 500.0;

/// "12 Nov 14:05 UTC (3 h ago)", or "unknown" for 0.
pub fn when(unix: u64, now: u64) -> String {
    if unix == 0 {
        return "unknown".into();
    }
    let day = fmt::last_played(unix, now);
    format!("{day} {} UTC ({})", fmt::clock(unix, 0), fmt::ago(unix, now))
}

/// The "saves differ" dialog. Shown while `state.cloud.conflict` is set; the game does not start until the
/// user chooses. Closing it (Esc) is the same as "Cancel".
pub fn conflict_dialog(ctx: &Context, client: &mut Client) {
    let Some(c) = client.state().cloud.conflict.clone() else { return };
    let now = fmt::now_unix();
    let mut choice = None;
    let modal = egui::Modal::new(Id::new("cloud-conflict"))
        .frame(
            egui::Frame::new()
                .fill(theme::MODAL_BG)
                .inner_margin(egui::Margin::same(theme::MODAL_PAD))
                .corner_radius(theme::BUTTON_RADIUS)
                .stroke(egui::Stroke::new(1.0, theme::WINDOW_BORDER)),
        )
        .show(ctx, |ui| {
            ui.set_width(DIALOG_W);
            theme::dialog_style(ui);
            ui.label(RichText::new("Steam Cloud: saves differ").font(theme::bold(22.0)).color(theme::TEXT));
            ui.add_space(10.0);
            let files = if c.files == 1 { "1 save file".to_string() } else { format!("{} save files", c.files) };
            ui.label(
                RichText::new(format!(
                    "{} has {files} that changed on this computer and in Steam Cloud since they were last in sync. \
                     Choose which ones to keep. The game starts after you choose.",
                    c.game
                ))
                .color(theme::TEXT),
            );
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!("This computer: changed {}", when(c.local_time, now))).color(theme::TEXT_DIM),
            );
            ui.label(RichText::new(format!("Steam Cloud: changed {}", when(c.cloud_time, now))).color(theme::TEXT_DIM));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Keeping the cloud's saves first copies yours to Kettle's cloud-backups folder.")
                    .size(12.0)
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(14.0);
            widgets::button_row(ui, |ui| {
                if widgets::button(ui, "Keep cloud saves", ButtonStyle::Primary, true).clicked() {
                    choice = Some(CloudChoice::KeepCloud);
                }
                if widgets::button(ui, "Keep this computer's saves", ButtonStyle::Secondary, true).clicked() {
                    choice = Some(CloudChoice::KeepLocal);
                }
                if widgets::button(ui, "Cancel", ButtonStyle::Secondary, true).clicked() {
                    choice = Some(CloudChoice::Cancel);
                }
            });
        });
    if choice.is_none() && modal.should_close() {
        choice = Some(CloudChoice::Cancel);
    }
    if let Some(choice) = choice {
        client.send(Command::CloudResolve(c.app, choice));
    }
}

/// The global switch. Put it under a section header such as "Steam Cloud".
pub fn settings_row(ui: &mut Ui, client: &mut Client) {
    let mut on = client.state().settings.cloud_sync;
    widgets::setting_row(
        ui,
        "Steam Cloud sync",
        "Download your saves before a game starts and upload them after it ends. Needs a Steam connection. \
         Each game can switch it off in its properties.",
        |ui| {
            widgets::toggle(ui, &mut on);
        },
    );
    if on != client.state().settings.cloud_sync {
        client.send(Command::SetCloudSync(on));
    }
}

/// The per-game switch for a game's properties. Greyed note when the global switch is off.
pub fn game_row(ui: &mut Ui, client: &mut Client, app: AppId) {
    let global = client.state().settings.cloud_sync;
    let mut on = !client.state().cloud.games.off.contains(&app.0.to_string());
    let desc = if global {
        "Sync this game's saves with Steam Cloud. Games without cloud saves are not affected."
    } else {
        "Steam Cloud sync is switched off in Settings, so this has no effect for now."
    };
    widgets::setting_row(ui, "Steam Cloud sync for this game", desc, |ui| {
        widgets::toggle(ui, &mut on);
    });
    if on != !client.state().cloud.games.off.contains(&app.0.to_string()) {
        client.send(Command::SetGameCloud(app, on));
    }
}

/// A small line for the game page: "Syncing saves..." while a sync runs, else the last result (if any).
/// Draws nothing when there is nothing to say.
pub fn status(ui: &mut Ui, client: &Client, app: AppId) {
    let cloud = &client.state().cloud;
    match cloud.active.get(&app) {
        Some(CloudPhase::Downloading) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Syncing...").size(13.0).color(theme::TEXT_DIM));
            });
            ui.ctx().request_repaint();
        }
        Some(CloudPhase::Uploading) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Uploading saves...").size(13.0).color(theme::TEXT_DIM));
            });
            ui.ctx().request_repaint();
        }
        None => {
            if let Some(text) = cloud.last.get(&app) {
                ui.label(RichText::new(text).size(12.0).color(theme::TEXT_DIM));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_naturally() {
        assert_eq!(when(0, 1_700_000_000), "unknown");
        let now = 1_700_000_000;
        let s = when(now - 7200, now);
        assert!(s.starts_with("Today ") && s.ends_with("UTC (2 h ago)"), "{s}");
    }
}
