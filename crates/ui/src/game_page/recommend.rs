//! "You've played for N hours. Would you recommend this game to other players?": the box the official client
//! shows under the tab box once a game has been played for a while. Yes / No open the review page of the store
//! in the browser (Kettle never signs in and cannot post a review itself); "Maybe Later" hides the box until
//! Kettle is restarted.

use crate::{anim, steam_icons, theme};
use egui::{Color32, Id, Margin, Sense, Ui, Vec2};
use sr_client::{Client, Command};
use sr_model::Game;

/// Played at least this long before the box appears.
pub const MIN_MINUTES: u32 = 120;

// Measured on the official client: box #434952, buttons #4a6172 with text #85bce2.
const BOX_BG: Color32 = Color32::from_rgb(0x43, 0x49, 0x52);
const BTN_BG: Color32 = Color32::from_rgb(0x4a, 0x61, 0x72);
const BTN_HOVER: Color32 = Color32::from_rgb(0x58, 0x74, 0x88);
const BTN_TEXT: Color32 = Color32::from_rgb(0x85, 0xbc, 0xe2);
const LEAD: Color32 = Color32::from_rgb(0x93, 0xad, 0xc2);
const ASK: Color32 = Color32::from_rgb(0xc9, 0xcf, 0xd6);

/// Whether the box is due for this game: a Steam game (not a shortcut) played long enough and not dismissed.
pub fn due(game: &Game, dismissed: bool) -> bool {
    game.app_id.0 & 0x8000_0000 == 0 && game.playtime_minutes >= MIN_MINUTES && !dismissed
}

/// "You've played for 368 hours" (rounded to whole hours).
pub fn lead_text(minutes: u32) -> String {
    let h = (minutes + 30) / 60;
    format!("You've played for {h} hour{}", if h == 1 { "" } else { "s" })
}

pub fn show(ui: &mut Ui, client: &mut Client, game: &Game) {
    let id = Id::new(("recommend-dismissed", game.app_id.0));
    let dismissed = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if !due(game, dismissed) {
        return;
    }
    let mut answer: Option<&str> = None;
    egui::Frame::new().fill(BOX_BG).corner_radius(2.0).inner_margin(Margin::symmetric(23, 15)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(egui::RichText::new(lead_text(game.playtime_minutes)).font(theme::regular(15.0)).color(LEAD));
                ui.label(
                    egui::RichText::new("Would you recommend this game to\nother players?")
                        .font(theme::regular(15.0))
                        .color(ASK),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (label, kind, w) in
                    [("Maybe Later", None, 108.0), ("No", Some(false), 86.0), ("Yes", Some(true), 90.0)]
                {
                    ui.add_space(7.0);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 36.0), Sense::click());
                    let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
                    ui.painter().rect_filled(rect, 2.0, BTN_BG.lerp_to_gamma(BTN_HOVER, t));
                    let g = ui.painter().layout_no_wrap(label.to_string(), theme::regular(14.0), BTN_TEXT);
                    let icon = if kind.is_some() { 22.0 } else { 0.0 };
                    let x = rect.center().x - (g.size().x + icon) / 2.0;
                    if let Some(up) = kind {
                        steam_icons::thumb(ui.painter(), egui::pos2(x + 7.0, rect.center().y), 14.0, up, BTN_TEXT);
                    }
                    ui.painter().galley(egui::pos2(x + icon, rect.center().y - g.size().y / 2.0), g, BTN_TEXT);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        answer = Some(label);
                    }
                }
            });
        });
    });
    if let Some(label) = answer {
        ui.data_mut(|d| d.insert_temp(id, true));
        if label != "Maybe Later" {
            client.send(Command::OpenUrl(format!(
                "https://store.steampowered.com/app/{}/#app_reviews_hash",
                game.app_id.0
            )));
        }
    }
    ui.add_space(24.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use sr_model::AppId;

    fn game(app: u32, minutes: u32) -> Game {
        let mut g = Game::new(AppId(app), "G".into());
        g.playtime_minutes = minutes;
        g
    }

    #[test]
    fn shows_after_enough_play_time_only() {
        assert!(!due(&game(440, 119), false));
        assert!(due(&game(440, 120), false));
        assert!(!due(&game(440, 5000), true));
        assert!(!due(&game(0x8000_0001, 5000), false), "shortcuts have no store page");
    }

    #[test]
    fn lead_text_rounds_hours() {
        assert_eq!(lead_text(60), "You've played for 1 hour");
        assert_eq!(lead_text(22_080), "You've played for 368 hours");
        assert_eq!(lead_text(150), "You've played for 3 hours");
    }
}
