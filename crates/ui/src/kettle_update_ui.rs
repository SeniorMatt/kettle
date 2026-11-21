//! Kettle's own update: the settings rows and a small banner.
//!
//! Hooks for the pages: [`settings_rows`] in Settings (General or About), [`banner`] above the library or in
//! the status bar. Everything goes through `Command::KettleUpdate`.

use crate::widgets::{self, ButtonStyle};
use crate::{fmt, theme};
use egui::{RichText, Ui};
use sr_client::{Client, Command, KettleUpdateCmd, KettleUpdateStatus};

/// One line about the state of the update, for rows and tooltips.
pub fn status_text(status: &KettleUpdateStatus, can_install: bool) -> String {
    let current = env!("CARGO_PKG_VERSION");
    match status {
        KettleUpdateStatus::Idle | KettleUpdateStatus::UpToDate if !can_install => {
            format!("Kettle {current}. Updating itself needs the AppImage version.")
        }
        KettleUpdateStatus::Idle => format!("Kettle {current}"),
        KettleUpdateStatus::Checking => "Looking for a new version...".to_string(),
        KettleUpdateStatus::UpToDate => format!("Kettle {current} is the newest version."),
        KettleUpdateStatus::Available(u) => format!("Kettle {} is available ({}).", u.version, fmt::bytes(u.size)),
        KettleUpdateStatus::Downloading { version, done, total } => {
            format!("Downloading Kettle {version}: {} of {}", fmt::bytes(*done), fmt::bytes(*total))
        }
        KettleUpdateStatus::Ready { version } => format!("Kettle {version} is installed. Restart to use it."),
        KettleUpdateStatus::Failed(e) => e.clone(),
    }
}

/// "Check for Kettle updates" and the current state with the matching button.
pub fn settings_rows(ui: &mut Ui, client: &mut Client) {
    let mut on = client.state().settings.check_kettle_updates;
    let desc = "Looks at Kettle's releases on GitHub when it starts and every 6 hours. Only a request for the list of \
                releases is sent, nothing about you. Nothing is installed without your OK.";
    widgets::setting_row(ui, "Check for Kettle updates", desc, |ui| {
        if widgets::toggle(ui, &mut on).changed() {
            client.send(Command::KettleUpdate(KettleUpdateCmd::SetEnabled(on)));
        }
    });
    let ku = client.state().kettle_update.clone();
    let text = status_text(&ku.status, ku.can_install);
    widgets::setting_row(ui, "Kettle version", &text, |ui| match &ku.status {
        KettleUpdateStatus::Available(_) => {
            if widgets::button(ui, "Update now", ButtonStyle::Primary, ku.can_install).clicked() {
                client.send(Command::KettleUpdate(KettleUpdateCmd::Install));
            }
        }
        KettleUpdateStatus::Ready { .. } => {
            if widgets::button(ui, "Restart now", ButtonStyle::Primary, true).clicked() {
                client.send(Command::KettleUpdate(KettleUpdateCmd::Restart));
            }
        }
        KettleUpdateStatus::Checking | KettleUpdateStatus::Downloading { .. } => {
            widgets::button(ui, "Working...", ButtonStyle::Secondary, false);
        }
        _ => {
            if widgets::button(ui, "Check now", ButtonStyle::Secondary, ku.can_install).clicked() {
                client.send(Command::KettleUpdate(KettleUpdateCmd::Check));
            }
        }
    });
}

/// A one-line bar: "Kettle X is available [Update now] [Later]", the download progress, or "Restart now".
/// Draws nothing (and returns false) in every other state. Call it where a bar fits, e.g. above the page.
#[allow(clippy::collapsible_match)]
pub fn banner(ui: &mut Ui, client: &mut Client) -> bool {
    let ku = client.state().kettle_update.clone();
    let show = matches!(
        ku.status,
        KettleUpdateStatus::Available(_) | KettleUpdateStatus::Downloading { .. } | KettleUpdateStatus::Ready { .. }
    );
    if !show {
        return false;
    }
    egui::Frame::new().fill(theme::BG_ACTION_BAR).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let text = status_text(&ku.status, ku.can_install);
            ui.label(RichText::new(text).font(theme::regular(13.0)).color(theme::TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match &ku.status {
                KettleUpdateStatus::Available(_) => {
                    if widgets::button(ui, "Update now", ButtonStyle::Primary, true).clicked() {
                        client.send(Command::KettleUpdate(KettleUpdateCmd::Install));
                    }
                    if widgets::button(ui, "Later", ButtonStyle::Secondary, true).clicked() {
                        client.send(Command::KettleUpdate(KettleUpdateCmd::Dismiss));
                    }
                }
                KettleUpdateStatus::Downloading { done, total, .. } => {
                    let f = if *total > 0 { *done as f32 / *total as f32 } else { 0.0 };
                    widgets::progress_bar(ui, 160.0, 6.0, f, theme::ACCENT);
                }
                KettleUpdateStatus::Ready { .. } => {
                    if widgets::button(ui, "Restart now", ButtonStyle::Primary, true).clicked() {
                        client.send(Command::KettleUpdate(KettleUpdateCmd::Restart));
                    }
                }
                _ => {}
            });
        });
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_name_the_state() {
        assert!(status_text(&KettleUpdateStatus::Checking, true).contains("Looking"));
        assert!(status_text(&KettleUpdateStatus::Idle, false).contains("AppImage"));
        assert!(status_text(&KettleUpdateStatus::UpToDate, true).contains("newest"));
        let r = KettleUpdateStatus::Ready { version: "9.9.9".into() };
        assert!(status_text(&r, true).contains("Restart"));
        assert_eq!(status_text(&KettleUpdateStatus::Failed("no".into()), true), "no");
    }
}
