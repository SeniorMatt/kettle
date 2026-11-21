//! The ProtonDB rating of a game: a small coloured label for the game page.
//!
//! Hook: call [`badge`] on the game page for games without a native Linux build (`native_linux` = the store
//! page lists Linux, or the game has a Linux depot). It asks the client for the rating once and draws nothing
//! while there is none.

use crate::theme;
use egui::{Color32, RichText, Ui};
use sr_client::{Client, Command, Load, ProtonTier, protondb_page_url};
use sr_model::AppId;

/// The colour of a tier, from the theme's own colours.
pub fn tier_color(t: ProtonTier) -> Color32 {
    match t {
        ProtonTier::Platinum => theme::ACCENT,
        ProtonTier::Gold => theme::RUNNING,
        ProtonTier::Silver => theme::TEXT,
        ProtonTier::Bronze => theme::TEXT_DIM,
        ProtonTier::Borked => theme::ERROR_TEXT,
        ProtonTier::Pending => theme::TEXT_UNINSTALLED,
    }
}

/// "ProtonDB: Gold" with a tooltip; clicking opens the game's ProtonDB page. Returns true when something was drawn.
pub fn badge(ui: &mut Ui, client: &mut Client, app: AppId, native_linux: bool) -> bool {
    if native_linux {
        return false;
    }
    match client.state().protondb.get(&app.0) {
        None => {
            client.send(Command::ProtonDb { app, native_linux });
            false
        }
        Some(Load::Ready(Some(r))) => {
            let tip = format!("{}\n{}\nClick to open ProtonDB", r.summary(), r.tier.meaning());
            let (tier, label) = (r.tier, r.tier.label());
            let resp = ui
                .horizontal(|ui| {
                    ui.label(RichText::new("ProtonDB").font(theme::regular(13.0)).color(theme::TEXT_DIM));
                    ui.label(RichText::new(label).font(theme::bold(13.0)).color(tier_color(tier)));
                })
                .response
                .interact(egui::Sense::click())
                .on_hover_text(tip)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.clicked() {
                client.send(Command::OpenUrl(protondb_page_url(app.0)));
            }
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tier_has_its_own_look() {
        let all = [
            ProtonTier::Platinum,
            ProtonTier::Gold,
            ProtonTier::Silver,
            ProtonTier::Bronze,
            ProtonTier::Borked,
            ProtonTier::Pending,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(tier_color(*a), tier_color(*b));
            }
        }
    }
}
