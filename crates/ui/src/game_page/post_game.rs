//! The boxes between the tab box and the news columns, as in the official client: "Would you recommend this
//! game" (after some play time) and the post-game summary with the screenshots of the last session.

use crate::textures::{Art, Textures};
use crate::{anim, theme, widgets};
use egui::{Color32, Id, Pos2, Rect, Sense, Ui, Vec2};
use egui_phosphor::regular as icon;
use sr_client::{Client, Command};
use sr_model::Game;

/// The question appears after this much play time.
pub const RECOMMEND_MIN_MINUTES: u32 = 60;
/// Right margin of the page's boxes (the left one is 20).
const RIGHT: f32 = 23.0;

/// "You've played for 368 hours".
pub fn played_text(minutes: u32) -> String {
    let h = minutes / 60;
    format!("You've played for {h} {}", if h == 1 { "hour" } else { "hours" })
}

/// Whether the box is shown for this game now.
pub fn recommend_visible(client: &Client, game: &Game) -> bool {
    game.playtime_minutes >= RECOMMEND_MIN_MINUTES && !client.state().gamepage.recommend_hidden.contains(&game.app_id.0)
}

pub fn summary_visible(client: &Client, game: &Game) -> bool {
    client.state().gamepage.post_game.contains_key(&game.app_id.0)
}

/// A row of the page's width (20 px in from the left, `RIGHT` from the right) of the given height.
fn row(ui: &mut Ui, h: f32) -> Rect {
    let w = ui.available_width() - 20.0 - RIGHT;
    ui.horizontal(|ui| {
        ui.add_space(20.0);
        ui.allocate_exact_size(Vec2::new(w, h), Sense::hover()).0
    })
    .inner
}

pub fn recommend(ui: &mut Ui, client: &mut Client, game: &Game) {
    let rect = row(ui, theme::REC_H);
    let app = game.app_id;
    ui.painter().rect_filled(rect, 2.0, theme::REC_BG);
    let title = ui.painter().layout_no_wrap(played_text(game.playtime_minutes), theme::regular(17.0), theme::REC_TITLE);
    let ty = rect.top() + 14.0;
    let tx = rect.left() + 23.0;
    let body_w = (rect.width() - 282.0 - 20.0).clamp(120.0, 250.0);
    let body = ui.painter().layout(
        "Would you recommend this game to other players?".to_string(),
        theme::regular(17.0),
        theme::REC_BODY,
        body_w,
    );
    let th = title.size().y;
    ui.painter().galley(Pos2::new(tx, ty), title, theme::REC_TITLE);
    ui.painter().galley(Pos2::new(tx, ty + th), body, theme::REC_BODY);

    // Buttons from the right: Maybe Later 108, No 87, Yes 91 wide, 6 apart, 28 from the edge.
    let top = rect.top() + 15.0;
    let mut right = rect.right() - 28.0;
    let defs: [(&str, Option<&str>, f32); 3] =
        [("Maybe Later", None, 108.0), ("No", Some(icon::THUMBS_DOWN), 87.0), ("Yes", Some(icon::THUMBS_UP), 91.0)];
    for (label, glyph, w) in defs {
        let r = Rect::from_min_size(Pos2::new(right - w, top), Vec2::new(w, 36.0));
        right -= w + 6.0;
        let resp = ui.interact(r, Id::new(("recommend", label)), Sense::click());
        let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
        ui.painter().rect_filled(r, 2.0, theme::REC_BTN.lerp_to_gamma(theme::REC_BTN_HOVER, t));
        let p = ui.painter();
        match glyph {
            Some(g) => {
                p.text(
                    Pos2::new(r.left() + 20.0, r.center().y),
                    egui::Align2::CENTER_CENTER,
                    g,
                    theme::regular(20.0),
                    theme::REC_BTN_TEXT,
                );
                p.text(
                    Pos2::new(r.left() + 40.0, r.center().y),
                    egui::Align2::LEFT_CENTER,
                    label,
                    theme::regular(16.0),
                    theme::REC_BTN_TEXT,
                );
            }
            None => {
                p.text(r.center(), egui::Align2::CENTER_CENTER, label, theme::regular(16.0), theme::REC_BTN_TEXT);
            }
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            if glyph.is_some() {
                // The review itself is written on the store page; there is no review call in the client.
                client.send(Command::OpenUrl(format!("https://store.steampowered.com/app/{}#app_reviews_hash", app.0)));
            }
            client.send(Command::HideRecommend(app, glyph.is_some()));
        }
    }
}

/// "Today", "Yesterday" or "Nov 12".
fn day_title(end: u64) -> String {
    let now = crate::fmt::now_unix();
    let offset = sr_client::local_utc_offset(now);
    let (d, today) = ((end as i64 + offset).div_euclid(86_400), (now as i64 + offset).div_euclid(86_400));
    let label = super::sections::day_label(d, today);
    let mut out = String::new();
    for (i, w) in label.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let mut cs = w.chars();
        if let Some(c) = cs.next() {
            out.extend(c.to_uppercase());
            out.push_str(&cs.as_str().to_lowercase());
        }
    }
    out
}

fn chevron(ui: &Ui, c: Pos2, left: bool, color: Color32) {
    let s = if left { -1.0 } else { 1.0 };
    let pts = vec![c + Vec2::new(-s * 3.5, -7.0), c + Vec2::new(s * 3.5, 0.0), c + Vec2::new(-s * 3.5, 7.0)];
    ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(1.6, color)));
}

pub fn summary(ui: &mut Ui, client: &mut Client, tex: &mut Textures, game: &Game) {
    let Some(pg) = client.state().gamepage.post_game.get(&game.app_id.0).cloned() else { return };
    let app = game.app_id;
    let head = row(ui, 22.0);
    ui.painter().text(
        Pos2::new(head.left(), head.center().y),
        egui::Align2::LEFT_CENTER,
        "POST-GAME SUMMARY",
        theme::regular(14.0),
        theme::GP_SECTION_TITLE,
    );
    ui.add_space(4.0);
    let panel = row(ui, theme::POST_PANEL_H);
    ui.painter().rect_filled(panel, 2.0, theme::POST_PANEL_BG);

    let key = Id::new(("post-game-offset", app.0));
    let mut offset: usize = ui.data(|d| d.get_temp(key)).unwrap_or(0);
    let n = pg.shots.len();
    offset = offset.min(n.saturating_sub(1));
    let can_prev = offset > 0;
    let can_next = n > offset + 2;

    // < > X at the right end of the heading.
    let c = |dx: f32| Pos2::new(head.right() - dx, head.center().y);
    for (dx, which) in [(73.0, 0), (46.0, 1), (16.0, 2)] {
        let r = Rect::from_center_size(c(dx), Vec2::splat(24.0));
        let enabled = match which {
            0 => can_prev,
            1 => can_next,
            _ => true,
        };
        let resp = ui.interact(r, Id::new(("post-btn", which)), Sense::click());
        let color = if !enabled {
            theme::POST_ICON_OFF
        } else if resp.hovered() {
            Color32::WHITE
        } else {
            theme::POST_ICON
        };
        match which {
            0 => chevron(ui, c(dx), true, color),
            1 => chevron(ui, c(dx), false, color),
            _ => crate::steam_icons::window_close(ui.painter(), c(dx), color),
        }
        if enabled && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            match which {
                0 => offset -= 1,
                1 => offset += 1,
                _ => client.send(Command::ClosePostGame(app)),
            }
        }
    }
    ui.data_mut(|d| d.insert_temp(key, offset));

    let saved = ui.clip_rect();
    ui.set_clip_rect(panel.intersect(saved));
    ui.painter().text(
        Pos2::new(panel.left() + 20.0, panel.top() + 20.0),
        egui::Align2::LEFT_CENTER,
        day_title(pg.end),
        theme::regular(16.0),
        theme::POST_DAY,
    );
    let step = theme::POST_THUMB_W + theme::POST_THUMB_GAP;
    for (i, shot) in pg.shots.iter().enumerate().skip(offset) {
        let x = panel.left() + 20.0 + (i - offset) as f32 * step;
        if x > panel.right() {
            break;
        }
        let r =
            Rect::from_min_size(Pos2::new(x, panel.top() + 34.0), Vec2::new(theme::POST_THUMB_W, theme::POST_THUMB_H));
        let url = shot.display().to_string();
        match tex.get_web(client, &url) {
            Art::Ready(t, a) => widgets::image_cover(ui, r, t, Color32::WHITE.gamma_multiply(a)),
            Art::Loading => widgets::placeholder(ui, r),
            Art::Missing => ui.painter().rect_filled(r, 0.0, theme::BG_CAPSULE),
        }
        let resp = ui.interact(r.intersect(panel), Id::new(("post-shot", i)), Sense::click());
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            client.send(Command::OpenScreenshots(app));
        }
    }
    ui.set_clip_rect(saved);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn played_text_counts_whole_hours() {
        assert_eq!(played_text(60), "You've played for 1 hour");
        assert_eq!(played_text(368 * 60 + 12), "You've played for 368 hours");
    }

    #[test]
    fn day_titles_are_capitalised() {
        assert_eq!(day_title(crate::fmt::now_unix()), "Today");
        assert_eq!(day_title(0).chars().next(), Some('J'));
    }
}
