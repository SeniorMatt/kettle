//! The two columns below a game page's tab box, as in the official client: news (featured event and the
//! activity list) on the left; friends who play, additional content and achievements on the right. A
//! section without data is not drawn at all.

use crate::textures::{Art, Textures};
use crate::{anim, steam_icons, theme};
use egui::text::LayoutJob;
use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use sr_client::{Achievements, Client, Command, DlcEntry, Friend, NewsItem};
use sr_model::{AppId, Game};
use std::sync::Arc;

/// Page data is asked for again when the page has been open this long (the client caches it anyway).
const RELOAD_AFTER: f64 = 600.0;
/// Friends listed in the box (the rest is behind "View all").
const MAX_FRIENDS: usize = 4;
/// DLC rows listed.
const MAX_DLC: usize = 6;
/// Achievement icons in the row, including the "+N" tile.
const ICONS: usize = 5;

pub fn show(ui: &mut Ui, client: &mut Client, tex: &mut Textures, game: &Game) {
    let app = game.app_id;
    // Ask for the data once when the page opens (and now and then while it stays open).
    let key = egui::Id::new("game-page-loaded");
    let now = ui.input(|i| i.time);
    let last: Option<(AppId, f64)> = ui.data(|d| d.get_temp(key));
    if last.is_none_or(|(a, t)| a != app || now - t > RELOAD_AFTER) {
        client.send(Command::LoadGamePage(app));
        ui.data_mut(|d| d.insert_temp(key, (app, now)));
    }

    let gp = &client.state().gamepage;
    let news = gp.news(app).cloned();
    let ach = gp.achievements(app).cloned();
    let dlc = gp.dlc(app).cloned();
    let friends = gp.friends_playing(&client.state().friends, app, &game.name);
    let has_left = news.is_some();
    let has_right = !friends.is_empty() || ach.is_some() || dlc.is_some();
    if !has_left && !has_right {
        return;
    }
    let right_w = if has_right { theme::GP_RIGHT_W } else { 0.0 };
    let gap = if has_left && has_right { theme::GP_COL_GAP } else { 0.0 };
    let left_w = (ui.available_width() - right_w - gap).max(240.0);
    let mut cmds: Vec<Command> = Vec::new();
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        if let Some(news) = &news {
            ui.vertical(|ui| {
                ui.set_width(left_w);
                news_column(ui, app, news, left_w, &mut cmds);
            });
        }
        ui.add_space(gap);
        if has_right {
            ui.vertical(|ui| {
                ui.set_width(right_w);
                if !friends.is_empty() {
                    friends_box(ui, client, tex, &friends, &mut cmds);
                }
                if let Some(dlc) = &dlc {
                    ui.add_space(30.0);
                    dlc_box(ui, app, dlc, &mut cmds);
                }
                if let Some(ach) = &ach {
                    ui.add_space(30.0);
                    achievements_box(ui, client, tex, app, ach);
                }
            });
        }
    });
    for c in cmds {
        client.send(c);
    }
}

// ---- helpers -----------------------------------------------------------------

/// Wrapped text of at most `rows` rows, painted in the next free spot.
fn text(ui: &mut Ui, s: &str, font: FontId, color: Color32, width: f32, rows: usize) -> Rect {
    let mut job = LayoutJob::simple(s.to_string(), font, color, width);
    job.wrap.max_rows = rows;
    job.wrap.overflow_character = Some('\u{2026}');
    let g = ui.painter().layout_job(job);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, g.size().y), Sense::hover());
    ui.painter().galley(rect.min, g, color);
    rect
}

/// "ACTIVITY", "FRIENDS WHO PLAY", ...
fn heading(ui: &mut Ui, title: &str, width: f32) {
    text(ui, title, theme::regular(14.0), theme::GP_SECTION_TITLE, width, 1);
    ui.add_space(9.0);
}

/// The flat box of the right column.
fn boxed(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(theme::GP_BOX_BG)
        .stroke(Stroke::new(1.0, theme::GP_BOX_BORDER))
        .corner_radius(2.0)
        .inner_margin(egui::Margin::same(11))
        .show(ui, |ui| {
            ui.set_width(theme::GP_RIGHT_W - 24.0);
            add(ui);
        });
}

/// A right-aligned grey link; returns true when clicked.
fn link(ui: &mut Ui, label: &str, width: f32) -> bool {
    let g = ui.painter().layout(label.to_string(), theme::regular(14.0), theme::GP_LINK, width);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, g.size().y), Sense::click());
    let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    let color = theme::GP_LINK.lerp_to_gamma(theme::TEXT, t);
    let pos = Pos2::new(rect.right() - g.size().x, rect.top());
    ui.painter().galley_with_override_text_color(pos, g, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn avatar(ui: &Ui, client: &Client, tex: &mut Textures, rect: Rect, f: &Friend) {
    let p = ui.painter();
    p.rect_filled(rect, 2.0, theme::BG_CAPSULE);
    if let Some(u) = f.avatar_url.as_deref()
        && let Art::Ready(t, a) = tex.get_web(client, u)
    {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        p.image(t.id(), rect, uv, Color32::WHITE.gamma_multiply(a));
        return;
    }
    let initial = f.display_name().chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
    p.text(rect.center(), egui::Align2::CENTER_CENTER, initial, theme::bold(15.0), theme::TEXT_DIM);
}

// ---- left column ----------------------------------------------------------------

fn news_column(ui: &mut Ui, app: AppId, news: &Arc<Vec<NewsItem>>, w: f32, cmds: &mut Vec<Command>) {
    let latest = &news[0];
    featured(ui, app, latest, w, cmds);
    ui.add_space(38.0);
    heading(ui, "ACTIVITY", w);
    say_something(ui, w);
    ui.add_space(21.0);
    if link(ui, "View Latest News", w) {
        cmds.push(Command::OpenUrl(sr_client::news_page(app.0)));
    }
    ui.add_space(2.0);
    let now = crate::fmt::now_unix();
    let offset = sr_client::local_utc_offset(now);
    let today = day_of(now, offset);
    let mut group: Option<i64> = None;
    for item in news.iter() {
        let day = day_of(item.date, offset);
        if group != Some(day) {
            group = Some(day);
            ui.add_space(12.0);
            let (line, _) = ui.allocate_exact_size(Vec2::new(w, 1.0), Sense::hover());
            ui.painter().rect_filled(line, 0.0, theme::GP_DATE_LINE);
            ui.add_space(9.0);
            text(ui, &day_label(day, today), theme::regular(13.0), theme::GP_LINK, w, 1);
            ui.add_space(9.0);
        }
        news_row(ui, app, item, w, cmds);
    }
}

/// The field under ACTIVITY. Kettle has no call that posts to the activity feed, so Enter only clears the
/// field and says so.
fn say_something(ui: &mut Ui, w: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, theme::SAY_H), Sense::hover());
    ui.painter().rect_filled(rect, 2.0, theme::SAY_BG);
    let id = egui::Id::new("say-something");
    let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let hint = egui::RichText::new("Say something about this game to your friends...")
        .font(theme::regular(16.0))
        .italics()
        .color(theme::SAY_HINT);
    let resp = ui.put(
        rect,
        egui::TextEdit::singleline(&mut text)
            .hint_text(hint)
            .font(theme::regular(16.0))
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::symmetric(18, 8))
            .vertical_align(egui::Align::Center)
            .desired_width(w),
    );
    let posted = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty();
    let note_id = id.with("note");
    if posted {
        text.clear();
        ui.data_mut(|d| d.insert_temp(note_id, true));
    }
    ui.data_mut(|d| d.insert_temp(id, text));
    if ui.data(|d| d.get_temp::<bool>(note_id)).unwrap_or(false) {
        ui.add_space(4.0);
        note(ui, "Posting to your activity feed is not available in Kettle yet.", w);
    }
}

fn note(ui: &mut Ui, s: &str, w: f32) {
    text(ui, s, theme::regular(12.0), theme::GP_LINK, w, 1);
}

/// The card with the newest news item: a blue strip, then the kind of post and its title.
fn featured(ui: &mut Ui, app: AppId, item: &NewsItem, w: f32, cmds: &mut Vec<Command>) {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, theme::GP_FEATURED_H), Sense::click());
    let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    let p = ui.painter();
    let head = Rect::from_min_size(rect.min, Vec2::new(w, theme::GP_FEATURED_HEADER_H));
    p.rect_filled(rect, 2.0, theme::GP_FEATURED_BODY.lerp_to_gamma(theme::GP_BOX_BORDER, 0.5 * t));
    p.rect_filled(head, egui::CornerRadius { nw: 2, ne: 2, sw: 0, se: 0 }, theme::GP_FEATURED_HEADER);
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, theme::GP_FEATURED_BORDER), StrokeKind::Inside);
    p.text(
        Pos2::new(rect.left() + 13.0, head.center().y),
        egui::Align2::LEFT_CENTER,
        "FEATURED",
        theme::regular(14.0),
        theme::GP_FEATURED_TEXT,
    );
    let body_c = rect.left() + 38.0;
    let cy = head.bottom() + (rect.bottom() - head.bottom()) / 2.0;
    steam_icons::gear(p, Pos2::new(body_c, cy), 36.0, theme::GP_FEATURED_TYPE, theme::GP_FEATURED_BODY);
    let x = rect.left() + 75.0;
    let kind = if item.label.is_empty() { "NEWS".to_string() } else { item.label.to_uppercase() };
    let g = p.layout(kind, theme::regular(13.0), theme::GP_FEATURED_TYPE, (rect.right() - x - 12.0).max(40.0));
    let title = p.layout(
        item.title.clone(),
        theme::regular(19.0),
        theme::GP_FEATURED_TITLE,
        (rect.right() - x - 12.0).max(40.0),
    );
    let total = g.size().y + 6.0 + title.size().y.min(26.0);
    let y = cy - total / 2.0;
    let ty = y + g.size().y + 6.0;
    p.galley(Pos2::new(x, y), g, theme::GP_FEATURED_TYPE);
    p.galley(Pos2::new(x, ty), title, theme::GP_FEATURED_TITLE);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        cmds.push(Command::OpenUrl(item.url(app.0)));
    }
}

/// One entry of the activity list: title and a few lines of the text.
fn news_row(ui: &mut Ui, app: AppId, item: &NewsItem, w: f32, cmds: &mut Vec<Command>) {
    let tg = ui.painter().layout(item.title.clone(), theme::regular(16.0), theme::GP_FEATURED_TITLE, w);
    let mut job = LayoutJob::simple(item.text.clone(), theme::regular(13.0), theme::TEXT_DIM, w);
    job.wrap.max_rows = 2;
    job.wrap.overflow_character = Some('\u{2026}');
    let bg = ui.painter().layout_job(job);
    let has_text = !item.text.is_empty();
    let h = tg.size().y + if has_text { 4.0 + bg.size().y } else { 0.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), Sense::click());
    let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
    let title_color = theme::GP_FEATURED_TITLE.lerp_to_gamma(Color32::WHITE, t);
    let ty = rect.top() + tg.size().y + 4.0;
    ui.painter().galley_with_override_text_color(rect.min, tg, title_color);
    if has_text {
        ui.painter().galley(Pos2::new(rect.left(), ty), bg, theme::TEXT_DIM);
    }
    ui.add_space(14.0);
    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
        cmds.push(Command::OpenUrl(item.url(app.0)));
    }
}

// ---- right column ---------------------------------------------------------------

fn friends_box(ui: &mut Ui, client: &Client, tex: &mut Textures, friends: &[Friend], cmds: &mut Vec<Command>) {
    let w = theme::GP_RIGHT_W;
    heading(ui, "FRIENDS WHO PLAY", w);
    boxed(ui, |ui| {
        let iw = w - 24.0;
        let n = friends.len();
        let msg = if n == 1 {
            "1 friend is playing right now".to_string()
        } else {
            format!("{n} friends are playing right now")
        };
        text(ui, &msg, theme::regular(14.0), theme::GP_BOX_TEXT, iw, 2);
        ui.add_space(10.0);
        for f in friends.iter().take(MAX_FRIENDS) {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(iw, 32.0), Sense::hover());
            avatar(ui, client, tex, Rect::from_min_size(rect.min, Vec2::splat(32.0)), f);
            let x = rect.left() + 40.0;
            let p = ui.painter();
            let one_row = |s: String, size: f32, color: Color32| {
                let mut job = LayoutJob::simple(s, theme::regular(size), color, iw - 40.0);
                job.wrap.max_rows = 1;
                job.wrap.overflow_character = Some('\u{2026}');
                p.layout_job(job)
            };
            let name = one_row(f.display_name(), 14.0, theme::GP_FRIEND_NAME);
            let rich = f
                .rich
                .as_deref()
                .filter(|r| !r.is_empty())
                .map(|r| one_row(r.to_string(), 13.0, theme::GP_FRIEND_RICH));
            let total = name.size().y.min(18.0) + rich.as_ref().map_or(0.0, |r| r.size().y.min(16.0));
            let y = rect.center().y - total / 2.0;
            let ny = y + name.size().y.min(18.0);
            p.galley(Pos2::new(x, y), name, theme::GP_FRIEND_NAME);
            if let Some(r) = rich {
                p.galley(Pos2::new(x, ny), r, theme::GP_FRIEND_RICH);
            }
            ui.add_space(8.0);
        }
        ui.add_space(12.0);
        if link(ui, "View all friends who play", iw) {
            cmds.push(Command::OpenFriends);
        }
    });
}

fn dlc_box(ui: &mut Ui, app: AppId, dlc: &Arc<Vec<DlcEntry>>, cmds: &mut Vec<Command>) {
    let w = theme::GP_RIGHT_W;
    heading(ui, "ADDITIONAL CONTENT", w);
    boxed(ui, |ui| {
        let iw = w - 24.0;
        for d in dlc.iter().take(MAX_DLC) {
            let tag = if d.installed { "Installed" } else { "" };
            let tag_w = if tag.is_empty() { 0.0 } else { 62.0 };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(iw, 22.0), Sense::click());
            let t = anim::animate_bool(ui.ctx(), resp.id.with("h"), resp.hovered(), anim::FAST);
            let color = theme::GP_BOX_TEXT.lerp_to_gamma(Color32::WHITE, t);
            let mut job = LayoutJob::simple_singleline(d.name.clone(), theme::regular(13.0), color);
            job.wrap.max_width = iw - tag_w;
            job.wrap.max_rows = 1;
            job.wrap.overflow_character = Some('\u{2026}');
            let g = ui.painter().layout_job(job);
            ui.painter().galley(Pos2::new(rect.left(), rect.center().y - g.size().y / 2.0), g, color);
            if !tag.is_empty() {
                ui.painter().text(
                    Pos2::new(rect.right(), rect.center().y),
                    egui::Align2::RIGHT_CENTER,
                    tag,
                    theme::regular(12.0),
                    theme::GP_FRIEND_RICH,
                );
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                cmds.push(Command::OpenUrl(format!("https://store.steampowered.com/app/{}", d.appid)));
            }
        }
        if dlc.len() > MAX_DLC {
            ui.add_space(8.0);
            if link(ui, &format!("View all {} DLC", dlc.len()), iw) {
                cmds.push(Command::OpenUrl(format!("https://store.steampowered.com/dlc/{}", app.0)));
            }
        }
    });
}

fn achievements_box(ui: &mut Ui, client: &Client, tex: &mut Textures, app: AppId, a: &Arc<Achievements>) {
    let w = theme::GP_RIGHT_W;
    heading(ui, "ACHIEVEMENTS", w);
    boxed(ui, |ui| {
        let iw = w - 24.0;
        let msg = format!("You've unlocked {}/{} ({}%)", a.unlocked, a.total, a.percent());
        text(ui, &msg, theme::regular(14.0), theme::GP_BOX_TEXT, iw, 2);
        ui.add_space(9.0);
        let (bar, _) = ui.allocate_exact_size(Vec2::new(iw, 6.0), Sense::hover());
        ui.painter().rect_filled(bar, 2.0, theme::BG_PANEL_DARK);
        let fill = Rect::from_min_size(
            bar.min,
            Vec2::new(bar.width() * a.unlocked as f32 / a.total.max(1) as f32, bar.height()),
        );
        ui.painter().rect_filled(fill, 2.0, theme::ACCENT);
        ui.add_space(12.0);
        let unlocked: Vec<_> = a.list.iter().filter(|x| x.unlocked).collect();
        if unlocked.is_empty() {
            return;
        }
        let size = 28.0;
        let gap = (iw - size * ICONS as f32) / (ICONS as f32 - 1.0);
        let more = unlocked.len().saturating_sub(ICONS);
        let shown = if more > 0 { ICONS - 1 } else { unlocked.len().min(ICONS) };
        let (row, _) = ui.allocate_exact_size(Vec2::new(iw, size), Sense::hover());
        for (i, ach) in unlocked.iter().take(shown).enumerate() {
            let r = Rect::from_min_size(Pos2::new(row.left() + i as f32 * (size + gap), row.top()), Vec2::splat(size));
            ui.painter().rect_filled(r, 2.0, theme::PAGE_ICON_BTN_BG);
            if let Some(url) = ach.icon_url(app.0)
                && let Art::Ready(t, al) = tex.get_web(client, &url)
            {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                ui.painter().image(t.id(), r, uv, Color32::WHITE.gamma_multiply(al));
            }
            let resp = ui.interact(r, egui::Id::new(("ach-icon", i)), Sense::hover());
            if !ach.desc.is_empty() {
                resp.on_hover_text(format!("{}\n{}", ach.display, ach.desc));
            } else {
                resp.on_hover_text(&ach.display);
            }
        }
        if more > 0 {
            let r =
                Rect::from_min_size(Pos2::new(row.left() + shown as f32 * (size + gap), row.top()), Vec2::splat(size));
            ui.painter().rect_filled(r, 2.0, theme::PAGE_ICON_BTN_BG);
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                format!("+{more}"),
                theme::regular(13.0),
                theme::GP_BOX_TEXT,
            );
        }
    });
}

// ---- dates ---------------------------------------------------------------------

/// Days since 1970-01-01 in local time.
fn day_of(unix: u64, offset: i64) -> i64 {
    (unix as i64 + offset).div_euclid(86_400)
}

/// "TODAY", "YESTERDAY" or "NOV 12" (with the year when it is not this year).
pub(super) fn day_label(day: i64, today: i64) -> String {
    match today - day {
        0 => return "TODAY".into(),
        1 => return "YESTERDAY".into(),
        _ => {}
    }
    let (y, m, d) = civil(day);
    let (ty, _, _) = civil(today);
    const MONTHS: [&str; 12] = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
    let month = MONTHS[(m as usize - 1).min(11)];
    if y == ty { format!("{month} {d}") } else { format!("{month} {d}, {y}") }
}

/// (year, month, day) of a day count since 1970-01-01 (proleptic Gregorian).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(20_412), (2025, 11, 20));
    }

    #[test]
    fn day_labels() {
        assert_eq!(day_label(100, 100), "TODAY");
        assert_eq!(day_label(99, 100), "YESTERDAY");
        // 2024-03-05 seen from 2024-03-20, and from 2025.
        assert_eq!(day_label(19_787, 19_802), "MAR 5");
        assert_eq!(day_label(19_787, 20_200), "MAR 5, 2024");
    }

    #[test]
    fn local_day_follows_the_offset() {
        assert_eq!(day_of(86_399, 0), 0);
        assert_eq!(day_of(86_399, 3600), 1);
        assert_eq!(day_of(0, -3600), -1);
    }
}
