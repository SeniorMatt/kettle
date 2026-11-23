//! The panels that open over the game: Game Overview, Achievements, Notes, Guides, Friends, Browser, Settings.
use crate::{
    ACCENT, BAR_BG, BTN_HOVER, INGAME, INGAME_DIM, Layout, ONLINE, ONLINE_DIM, OverlayAction, OverlayData,
    OverlayFriend, OverlayState, PANEL_BG, PANEL_BG2, Panel, ROW_BG, TEXT, TEXT_DIM, icons, text,
};
use egui::{Align2, Color32, Id, Order, Painter, Pos2, Rect, Sense, TextureHandle, Ui, UiBuilder, Vec2, pos2, vec2};

const HOME: [(f32, f32, f32, f32); crate::PANEL_COUNT] = [
    (229.0, 168.0, 332.0, 556.0),  // overview
    (961.0, 455.0, 482.0, 457.0),  // achievements
    (642.0, 169.0, 499.0, 448.0),  // notes
    (330.0, 260.0, 332.0, 250.0),  // guides
    (1551.0, 163.0, 246.0, 627.0), // friends
    (700.0, 220.0, 560.0, 430.0),  // browser
    (640.0, 330.0, 360.0, 270.0),  // settings
];
const IDS: [&str; crate::PANEL_COUNT] =
    ["ov-overview", "ov-ach", "ov-notes", "ov-guides", "ov-friends", "ov-browser", "ov-settings"];

/// One open panel while it is being drawn: its origin and a painter clipped to it.
struct Pn<'a> {
    l: &'a Layout,
    o: Pos2,
    p: Painter,
}

impl Pn<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        self.o + vec2(x, y) * self.l.k
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.pt(x, y), vec2(w, h) * self.l.k)
    }
    #[allow(clippy::too_many_arguments)]
    fn text(&self, x: f32, y: f32, a: Align2, s: &str, size: f32, c: Color32, bold: bool) -> Rect {
        text(&self.p, self.l, self.pt(x, y), a, s, size, c, bold)
    }
}

pub fn all(
    ctx: &egui::Context,
    l: &Layout,
    data: &OverlayData,
    st: &mut OverlayState,
    actions: &mut Vec<OverlayAction>,
) {
    for (i, panel) in [
        Panel::Overview,
        Panel::Achievements,
        Panel::Notes,
        Panel::Guides,
        Panel::Friends,
        Panel::Browser,
        Panel::Settings,
    ]
    .iter()
    .enumerate()
    {
        if !st.open[i] {
            continue;
        }
        let (hx, hy, w, h) = HOME[i];
        let base = l.pt(hx, hy) + st.offsets[i] * l.k;
        // Keep the panel on screen.
        let size = vec2(w, h) * l.k;
        let pos = pos2(
            base.x.clamp(l.screen.min.x, (l.screen.max.x - size.x).max(l.screen.min.x)),
            base.y.clamp(l.screen.min.y, (l.screen.max.y - size.y).max(l.screen.min.y)),
        );
        let panel = *panel;
        egui::Area::new(Id::new(IDS[i])).order(Order::Middle).fade_in(false).fixed_pos(pos).show(ctx, |ui| {
            let (full, _) = ui.allocate_exact_size(size, Sense::hover());
            let bg = if panel == Panel::Achievements { PANEL_BG2 } else { PANEL_BG };
            ui.painter().rect_filled(full, 0.0, bg);
            let pn = Pn { l, o: full.min, p: ui.painter_at(full) };
            // Drag by the top strip.
            let drag_h = if panel == Panel::Friends { 52.0 } else { 56.0 };
            let drag = ui.interact(pn.rect(0.0, 0.0, w - 36.0, drag_h), Id::new((IDS[i], "drag")), Sense::drag());
            if drag.dragged() {
                st.offsets[i] += drag.drag_delta() / l.k;
            }
            match panel {
                Panel::Overview => overview(ui, &pn, data, st, actions),
                Panel::Achievements => achievements(ui, &pn, data, st, w),
                Panel::Notes => notes(ui, &pn, data, st, w, h),
                Panel::Guides => guides(ui, &pn, data, actions),
                Panel::Friends => friends(ui, &pn, data, st, actions, w, h),
                Panel::Browser => browser(ui, &pn, data, st, actions, w),
                Panel::Settings => settings(ui, &pn, st, actions),
            }
            // Close cross, top right (drawn last, on top of the content).
            let (cx, cy) = if panel == Panel::Friends { (w - 13.0, 12.0) } else { (w - 12.0, 12.0) };
            let cr = pn.rect(cx - 9.0, cy - 9.0, 18.0, 18.0);
            let resp = ui.interact(cr, Id::new((IDS[i], "close")), Sense::click());
            let col = if resp.hovered() { Color32::WHITE } else { Color32::from_rgb(0xa8, 0xae, 0xb5) };
            icons::cross(&pn.p, Rect::from_center_size(cr.center(), Vec2::splat(7.0 * l.k)), col, 1.4 * l.k);
            if resp.clicked() {
                st.open[i] = false;
            }
        });
    }
}

/// Scroll a list: wheel over `area` moves `st.scroll[i]` within 0..=max (points).
fn scroll(ui: &Ui, area: Rect, cur: &mut f32, max: f32) {
    if ui.rect_contains_pointer(area) {
        *cur -= ui.input(|i| i.smooth_scroll_delta.y);
    }
    *cur = cur.clamp(0.0, max.max(0.0));
}

fn scrollbar(pn: &Pn, area: Rect, cur: f32, content: f32) {
    if content <= area.height() {
        return;
    }
    let th = (area.height() / content * area.height()).max(20.0 * pn.l.k);
    let y = area.min.y + (cur / (content - area.height())) * (area.height() - th);
    pn.p.rect_filled(
        Rect::from_min_size(pos2(area.max.x + 3.0 * pn.l.k, y), vec2(4.0 * pn.l.k, th)),
        2.0 * pn.l.k,
        Color32::from_rgb(0x4b, 0x52, 0x5e),
    );
}

// ---- Game Overview -------------------------------------------------------------------------------------------

fn section(pn: &Pn, y: f32, w: f32, title: &str, icon: fn(&Painter, Rect, Color32)) {
    let hr = pn.rect(12.0, y, w - 24.0, 22.0);
    pn.p.rect_filled(hr, 0.0, BAR_BG);
    icons::grip(&pn.p, Rect::from_center_size(pn.pt(21.0, y + 11.0), Vec2::splat(8.0 * pn.l.k)), TEXT_DIM);
    pn.text(30.0, y + 11.0, Align2::LEFT_CENTER, title, 10.5, Color32::from_rgb(0xdc, 0xde, 0xdf), true);
    icon(
        &pn.p,
        Rect::from_center_size(pn.pt(w - 24.0, y + 11.0), Vec2::splat(11.0 * pn.l.k)),
        Color32::from_rgb(0xdc, 0xde, 0xdf),
    );
}

fn hours(min: u32) -> String {
    if min < 60 { format!("{} min", min) } else { format!("{:.1} hrs", min as f32 / 60.0) }
}

fn ago(ts: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(ts);
    let days = now.saturating_sub(ts) / 86400;
    match days {
        0 => "TODAY".into(),
        1 => "YESTERDAY".into(),
        n => format!("{n} DAYS AGO"),
    }
}

fn overview(ui: &mut Ui, pn: &Pn, data: &OverlayData, st: &mut OverlayState, actions: &mut Vec<OverlayAction>) {
    let w = 332.0;
    pn.text(12.0, 38.0, Align2::LEFT_CENTER, "Game Overview", 18.5, TEXT, true);
    let mut y = 58.0;

    section(pn, y, w, "PLAYTIME", icons::clock);
    y += 22.0;
    pn.p.rect_filled(pn.rect(12.0, y, w - 24.0, 46.0), 0.0, ROW_BG);
    pn.text(
        20.0,
        y + 14.0,
        Align2::LEFT_CENTER,
        &format!("Total Playtime: {}", hours(data.total_minutes)),
        9.8,
        TEXT,
        false,
    );
    pn.text(
        20.0,
        y + 32.0,
        Align2::LEFT_CENTER,
        &format!("This session: {}", hours(data.session_minutes)),
        9.8,
        TEXT_DIM,
        false,
    );
    y += 52.0;

    section(pn, y, w, "FRIENDS WHO ARE PLAYING", icons::people);
    y += 22.0;
    pn.p.rect_filled(pn.rect(12.0, y, w - 24.0, 36.0), 0.0, ROW_BG);
    let playing: Vec<&OverlayFriend> = data.friends.iter().filter(|f| f.in_game).take(9).collect();
    if playing.is_empty() {
        pn.text(20.0, y + 18.0, Align2::LEFT_CENTER, "No friends are playing right now", 9.8, TEXT_DIM, false);
    }
    for (i, f) in playing.iter().enumerate() {
        let r = pn.rect(20.0 + i as f32 * 29.0, y + 6.0, 24.0, 24.0);
        avatar(ui, &pn.p, r, f, pn.l.k, INGAME);
    }
    y += 42.0;

    section(pn, y, w, "YOUR ACHIEVEMENTS", icons::trophy);
    y += 22.0;
    pn.p.rect_filled(pn.rect(12.0, y, w - 24.0, 74.0), 0.0, ROW_BG);
    match &data.achievements {
        Some(a) => {
            if let Some((name, desc, ts)) = a.recent.first() {
                pn.text(20.0, y + 12.0, Align2::LEFT_CENTER, &ago(*ts), 7.5, TEXT, true);
                badge(&pn.p, pn.rect(20.0, y + 22.0, 40.0, 40.0), name, pn.l);
                pn.text(70.0, y + 31.0, Align2::LEFT_CENTER, &clip(name, 28), 11.5, TEXT, true);
                pn.text(70.0, y + 46.0, Align2::LEFT_CENTER, &clip(desc, 40), 9.0, TEXT_DIM, false);
                pn.text(
                    70.0,
                    y + 58.0,
                    Align2::LEFT_CENTER,
                    &format!("{} of {} unlocked", a.unlocked, a.total),
                    9.0,
                    TEXT_DIM,
                    false,
                );
            } else {
                pn.text(
                    20.0,
                    y + 37.0,
                    Align2::LEFT_CENTER,
                    &format!("{} of {} unlocked", a.unlocked, a.total),
                    9.8,
                    TEXT_DIM,
                    false,
                );
            }
        }
        None => {
            pn.text(20.0, y + 37.0, Align2::LEFT_CENTER, "No achievements for this game", 9.8, TEXT_DIM, false);
        }
    }
    y += 80.0;

    section(pn, y, w, "YOUR SCREENSHOTS", icons::screenshot);
    y += 22.0;
    pn.p.rect_filled(pn.rect(12.0, y, w - 24.0, 64.0), 0.0, ROW_BG);
    if flat_button(ui, pn, Id::new("ov-shot"), pn.rect(20.0, y + 16.0, 150.0, 32.0), "TAKE SCREENSHOT", 9.5) {
        actions.push(OverlayAction::TakeScreenshot);
    }
    pn.text(180.0, y + 32.0, Align2::LEFT_CENTER, "F12 in game", 9.0, TEXT_DIM, false);
    y += 70.0;

    section(pn, y, w, "TOP GUIDES", icons::book_small);
    y += 22.0;
    pn.p.rect_filled(pn.rect(12.0, y, w - 24.0, 44.0), 0.0, ROW_BG);
    if link(ui, pn, Id::new("ov-guides-link"), pn.pt(20.0, y + 22.0), "Browse community guides", 10.0) {
        actions.push(OverlayAction::OpenUrl(format!("https://steamcommunity.com/app/{}/guides/", data.app_id)));
    }
    let _ = st;
}

fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}...", s.chars().take(n - 3).collect::<String>().trim_end())
    }
}

/// Flat dark button with caps text, like Steam's small overlay buttons.
fn flat_button(ui: &mut Ui, pn: &Pn, id: Id, r: Rect, label: &str, size: f32) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    pn.p.rect_filled(r, 2.0 * pn.l.k, if resp.hovered() { Color32::from_rgb(0x4a, 0x51, 0x5e) } else { BAR_BG });
    text(&pn.p, pn.l, r.center(), Align2::CENTER_CENTER, label, size, TEXT, true);
    resp.clicked()
}

/// Text link that brightens on hover.
fn link(ui: &mut Ui, pn: &Pn, id: Id, pos: Pos2, label: &str, size: f32) -> bool {
    let g = pn.p.layout_no_wrap(label.to_string(), pn.l.font(size, false), TEXT);
    let r = Rect::from_min_size(pos2(pos.x, pos.y - g.size().y / 2.0), g.size());
    let resp = ui.interact(r.expand(3.0), id, Sense::click());
    let c = if resp.hovered() { Color32::WHITE } else { ACCENT };
    pn.p.galley(r.min, pn.p.layout_no_wrap(label.to_string(), pn.l.font(size, false), c), c);
    resp.clicked()
}

// ---- avatars -------------------------------------------------------------------------------------------------

fn avatar_texture(ui: &Ui, f: &OverlayFriend) -> Option<TextureHandle> {
    let (n, px) = f.avatar.as_ref()?;
    if *n == 0 || px.len() != n * n * 4 {
        return None;
    }
    let id = Id::new(("kettle-ov-avatar", f.steam_id, px.len(), px.first().copied(), px.last().copied()));
    if let Some(t) = ui.ctx().data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let img = egui::ColorImage::from_rgba_unmultiplied([*n, *n], px);
    let t = ui.ctx().load_texture(format!("ov-av-{}", f.steam_id), img, egui::TextureOptions::LINEAR);
    ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

/// Square avatar with a 2 px frame in the status colour (Steam style); a letter tile when there is no image.
fn avatar(ui: &Ui, p: &Painter, r: Rect, f: &OverlayFriend, k: f32, frame: Color32) {
    p.rect_filled(r, 0.0, frame);
    let inner = r.shrink(1.5 * k);
    match avatar_texture(ui, f) {
        Some(t) => {
            p.image(t.id(), inner, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            let h = f.steam_id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40;
            let c =
                Color32::from_rgb(40 + (h & 0x3f) as u8, 50 + ((h >> 6) & 0x3f) as u8, 70 + ((h >> 12) & 0x3f) as u8);
            p.rect_filled(inner, 0.0, c);
            let ch = f.name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
            p.text(
                inner.center(),
                Align2::CENTER_CENTER,
                ch,
                egui::FontId::proportional(inner.height() * 0.55),
                Color32::from_white_alpha(200),
            );
        }
    }
}

/// Achievement badge: dark frame with a medal (the real achievement art is not part of the data).
fn badge(p: &Painter, r: Rect, name: &str, l: &Layout) {
    p.rect_filled(r, 0.0, Color32::from_rgb(0x11, 0x13, 0x18));
    p.rect_stroke(r, 0.0, egui::Stroke::new(1.0, Color32::from_rgb(0x3a, 0x40, 0x4b)), egui::StrokeKind::Inside);
    let _ = (name, l);
    icons::medal(
        p,
        Rect::from_center_size(r.center(), Vec2::splat(r.width() * 0.5)),
        Color32::from_rgb(0xc7, 0xcf, 0xd6),
        Color32::from_rgb(0x11, 0x13, 0x18),
    );
}

// ---- Achievements --------------------------------------------------------------------------------------------

fn achievements(ui: &mut Ui, pn: &Pn, data: &OverlayData, st: &mut OverlayState, w: f32) {
    let k = pn.l.k;
    pn.p.rect_filled(pn.rect(0.0, 0.0, w, 1.5), 0.0, ACCENT);
    pn.text(19.0, 40.0, Align2::LEFT_CENTER, "Achievements", 17.5, TEXT, true);
    let (un, total) = data.achievements.as_ref().map(|a| (a.unlocked, a.total)).unwrap_or((0, 0));
    let frac = if total == 0 { 0.0 } else { un as f32 / total as f32 };
    let bar = pn.rect(19.0, 57.0, w - 38.0, 33.0);
    pn.p.rect_filled(bar, 0.0, BAR_BG);
    pn.text(31.0, 69.0, Align2::LEFT_CENTER, &format!("{un} OF {total} ACHIEVEMENTS EARNED"), 9.3, TEXT, true);
    pn.text(w - 31.0, 69.0, Align2::RIGHT_CENTER, &format!("({:.0}%)", frac * 100.0), 9.3, TEXT, true);
    let track = pn.rect(31.0, 80.0, w - 62.0, 5.0);
    pn.p.rect_filled(track, 0.0, Color32::from_rgb(0x2b, 0x30, 0x39));
    pn.p.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * frac, track.height())), 0.0, ACCENT);

    // Tabs.
    let tabs = [("IN PROGRESS", 62.0, 90.0), ("MY ACHIEVEMENTS", 215.0, 100.0), ("GLOBAL ACHIEVEMENTS", 349.0, 130.0)];
    let _ = tabs;
    let spec: [(&str, f32); 3] = [("IN PROGRESS", 62.0), ("MY ACHIEVEMENTS", 159.0), ("GLOBAL ACHIEVEMENTS", 297.0)];
    let widths = [90.0, 118.0, 150.0];
    for (i, (label, x)) in spec.iter().enumerate() {
        let r = pn.rect(*x, 113.0, widths[i], 27.0);
        let resp = ui.interact(r, Id::new(("ov-ach-tab", i)), Sense::click());
        if st.ach_tab as usize == i {
            pn.p.rect_filled(r, 13.0 * k, BAR_BG);
        } else if resp.hovered() {
            pn.p.rect_filled(r, 13.0 * k, Color32::from_rgb(0x2a, 0x2f, 0x39));
        }
        text(&pn.p, pn.l, r.center(), Align2::CENTER_CENTER, label, 8.6, TEXT, true);
        if resp.clicked() {
            st.ach_tab = i as u8;
        }
    }
    let heading = ["In Progress", "My Achievements", "Global Achievements"][st.ach_tab.min(2) as usize];
    pn.text(16.0, 171.0, Align2::LEFT_CENTER, heading, 12.5, TEXT, false);
    let sr = pn.rect(w - 227.0, 155.0, 172.0, 32.0);
    pn.p.rect_filled(sr, 0.0, Color32::from_rgb(0x25, 0x2a, 0x30));
    text(&pn.p, pn.l, sr.min + vec2(8.0 * k, sr.height() / 2.0), Align2::LEFT_CENTER, "Search", 9.3, TEXT_DIM, false);

    // List.
    let list = pn.rect(16.0, 200.0, w - 32.0, 242.0);
    let row_h = 56.0;
    let items: &[(String, String, u64)] = data.achievements.as_ref().map(|a| a.recent.as_slice()).unwrap_or(&[]);
    let content = items.len() as f32 * row_h * k;
    scroll(ui, list, &mut st.scroll[Panel::Achievements as usize], content - list.height());
    let off = st.scroll[Panel::Achievements as usize];
    let clip_p = pn.p.with_clip_rect(list);
    if items.is_empty() {
        let msg = if total == 0 { "This game has no achievements" } else { "No recent achievements yet" };
        text(&clip_p, pn.l, list.min + vec2(4.0 * k, 24.0 * k), Align2::LEFT_CENTER, msg, 10.5, TEXT_DIM, false);
    }
    for (i, (name, desc, ts)) in items.iter().enumerate() {
        let r = Rect::from_min_size(list.min + vec2(0.0, i as f32 * row_h * k - off), vec2(list.width(), 52.0 * k));
        if !r.intersects(list) {
            continue;
        }
        clip_p.rect_filled(r, 0.0, ROW_BG);
        badge(&clip_p, Rect::from_min_size(r.min + vec2(5.0, 6.0) * k, Vec2::splat(40.0 * k)), name, pn.l);
        text(&clip_p, pn.l, r.min + vec2(52.0, 18.0) * k, Align2::LEFT_CENTER, name, 11.5, TEXT, true);
        text(&clip_p, pn.l, r.min + vec2(52.0, 34.0) * k, Align2::LEFT_CENTER, &clip(desc, 52), 9.0, TEXT_DIM, false);
        icons::grip(
            &clip_p,
            Rect::from_center_size(pos2(r.max.x - 18.0 * k, r.center().y), Vec2::splat(20.0 * k)),
            TEXT_DIM,
        );
        let _ = ts;
    }
    scrollbar(pn, list, off, content);
}

// ---- Notes ---------------------------------------------------------------------------------------------------

fn notes(ui: &mut Ui, pn: &Pn, data: &OverlayData, st: &mut OverlayState, w: f32, h: f32) {
    let k = pn.l.k;
    pn.p.rect_filled(pn.rect(0.0, 0.0, 126.0, h), 0.0, Color32::from_rgb(0x2a, 0x2e, 0x38));
    pn.text(20.0, 37.0, Align2::LEFT_CENTER, "NOTES", 11.0, ACCENT, true);
    pn.p.rect_filled(pn.rect(0.0, 66.0, 126.0, 30.0), 0.0, BAR_BG);
    let name = if data.game_name.is_empty() { "Notes".to_string() } else { clip(&data.game_name, 16) };
    pn.text(20.0, 81.0, Align2::LEFT_CENTER, &name, 10.0, TEXT, false);
    pn.p.rect_filled(pn.rect(126.0, 0.0, w - 126.0, 48.0), 0.0, ROW_BG);
    pn.text(142.0, 24.0, Align2::LEFT_CENTER, "Notes are kept while the game runs", 9.5, TEXT_DIM, false);
    let ed = pn.rect(126.0, 48.0, w - 126.0, h - 48.0);
    pn.p.rect_filled(ed, 0.0, Color32::from_rgb(0x17, 0x1b, 0x22));
    let inner = ed.shrink2(vec2(14.0 * k, 12.0 * k));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
    child.visuals_mut().override_text_color = Some(TEXT);
    child.add_sized(
        inner.size(),
        egui::TextEdit::multiline(&mut st.notes)
            .frame(egui::Frame::NONE)
            .font(pn.l.font(12.5, false))
            .hint_text("Write a note")
            .desired_width(inner.width()),
    );
}

// ---- Guides --------------------------------------------------------------------------------------------------

fn guides(ui: &mut Ui, pn: &Pn, data: &OverlayData, actions: &mut Vec<OverlayAction>) {
    pn.text(12.0, 38.0, Align2::LEFT_CENTER, "Guides", 18.5, TEXT, true);
    let items = [
        ("Browse community guides", "guides/"),
        ("Create a guide", "guides/edit/"),
        ("Game hub", ""),
        ("Discussions", "discussions/"),
    ];
    for (i, (label, path)) in items.iter().enumerate() {
        let y = 62.0 + i as f32 * 44.0;
        let r = pn.rect(12.0, y, 308.0, 38.0);
        let resp = ui.interact(r, Id::new(("ov-guide", i)), Sense::click());
        pn.p.rect_filled(r, 0.0, if resp.hovered() { BTN_HOVER } else { ROW_BG });
        pn.text(24.0, y + 19.0, Align2::LEFT_CENTER, label, 11.5, TEXT, true);
        if resp.clicked() {
            actions.push(OverlayAction::OpenUrl(format!("https://steamcommunity.com/app/{}/{}", data.app_id, path)));
        }
    }
}

// ---- Friends -------------------------------------------------------------------------------------------------

fn friends(
    ui: &mut Ui,
    pn: &Pn,
    data: &OverlayData,
    st: &mut OverlayState,
    actions: &mut Vec<OverlayAction>,
    w: f32,
    h: f32,
) {
    let k = pn.l.k;
    // Account row.
    pn.p.rect_filled(pn.rect(0.0, 0.0, w, 52.0), 0.0, Color32::from_rgb(0x2a, 0x30, 0x3a));
    let me = OverlayFriend { name: data.account_name.clone(), steam_id: 1, ..Default::default() };
    avatar(ui, &pn.p, pn.rect(8.0, 9.0, 36.0, 36.0), &me, k, Color32::from_rgb(0x57, 0x5f, 0x6b));
    let nm = pn.text(
        50.0,
        20.0,
        Align2::LEFT_CENTER,
        if data.account_name.is_empty() { "You" } else { &data.account_name },
        11.5,
        TEXT,
        false,
    );
    icons::chevron(&pn.p, pos2(nm.max.x + 7.0 * k, nm.center().y + 1.0), 2.8 * k, TEXT_DIM);
    pn.text(50.0, 36.0, Align2::LEFT_CENTER, &data.game_name, 9.2, INGAME, false);
    icons::gear(
        &pn.p,
        Rect::from_center_size(pn.pt(w - 13.0, 38.0), Vec2::splat(11.0 * k)),
        TEXT_DIM,
        Color32::from_rgb(0x2a, 0x30, 0x3a),
    );
    // FRIENDS bar.
    pn.p.rect_filled(pn.rect(0.0, 52.0, w, 25.0), 0.0, Color32::from_rgb(0x42, 0x49, 0x53));
    pn.text(10.0, 64.5, Align2::LEFT_CENTER, "FRIENDS", 8.8, Color32::from_rgb(0xc2, 0xc7, 0xcd), true);
    icons::search(
        &pn.p,
        Rect::from_center_size(pn.pt(w - 42.0, 64.5), Vec2::splat(10.0 * k)),
        Color32::from_rgb(0xc2, 0xc7, 0xcd),
    );

    let list = pn.rect(0.0, 83.0, w, h - 83.0 - 6.0);
    let in_game: Vec<&OverlayFriend> = data.friends.iter().filter(|f| f.in_game).collect();
    let online: Vec<&OverlayFriend> = data.friends.iter().filter(|f| !f.in_game && f.online).collect();
    let row_h = 34.5;
    let rows = in_game.len() + online.len();
    let heads = (!in_game.is_empty()) as usize + (!online.is_empty()) as usize;
    let content = (rows as f32 * row_h + heads as f32 * 27.0 + 8.0) * k;
    scroll(ui, list, &mut st.scroll[Panel::Friends as usize], content - list.height());
    let off = st.scroll[Panel::Friends as usize];
    let cp = pn.p.with_clip_rect(list);
    let mut y = list.min.y - off;
    if rows == 0 {
        text(
            &cp,
            pn.l,
            pos2(list.min.x + 12.0 * k, y + 18.0 * k),
            Align2::LEFT_CENTER,
            "No friends online",
            10.0,
            TEXT_DIM,
            false,
        );
    }
    for (title, group, ing) in [("In-Game", &in_game, true), ("Online Friends", &online, false)] {
        if group.is_empty() {
            continue;
        }
        let tr =
            text(&cp, pn.l, pos2(list.min.x + 12.0 * k, y + 13.0 * k), Align2::LEFT_CENTER, title, 10.2, TEXT, false);
        text(
            &cp,
            pn.l,
            pos2(tr.max.x + 4.0 * k, y + 13.0 * k),
            Align2::LEFT_CENTER,
            &format!("({})", group.len()),
            9.5,
            TEXT_DIM,
            false,
        );
        y += 27.0 * k;
        for f in group.iter() {
            let r = Rect::from_min_size(pos2(list.min.x, y), vec2(w * k, row_h * k));
            if r.intersects(list) {
                let resp = ui.interact(r.intersect(list), Id::new(("ov-friend", f.steam_id)), Sense::click());
                if resp.hovered() {
                    cp.rect_filled(r, 0.0, Color32::from_rgb(0x2c, 0x32, 0x3c));
                }
                let (c, d) = if ing { (INGAME, INGAME_DIM) } else { (ONLINE, ONLINE_DIM) };
                avatar(ui, &cp, Rect::from_min_size(r.min + vec2(10.0, 4.0) * k, Vec2::splat(28.0 * k)), f, k, c);
                text(&cp, pn.l, r.min + vec2(46.0, 12.5) * k, Align2::LEFT_CENTER, &clip(&f.name, 24), 10.8, c, false);
                text(&cp, pn.l, r.min + vec2(46.0, 25.5) * k, Align2::LEFT_CENTER, &clip(&f.status, 34), 8.6, d, false);
                if resp.clicked() {
                    actions.push(OverlayAction::OpenChat(f.steam_id));
                }
            }
            y += row_h * k;
        }
        y += 8.0 * k;
    }
    scrollbar(pn, Rect::from_min_max(list.min, pos2(list.max.x - 7.0 * k, list.max.y)), off, content);
}

// ---- Browser -------------------------------------------------------------------------------------------------

fn browser(ui: &mut Ui, pn: &Pn, data: &OverlayData, st: &mut OverlayState, actions: &mut Vec<OverlayAction>, w: f32) {
    let k = pn.l.k;
    pn.text(16.0, 30.0, Align2::LEFT_CENTER, "Web Browser", 18.5, TEXT, true);
    let field = pn.rect(16.0, 58.0, w - 32.0 - 120.0, 32.0);
    pn.p.rect_filled(field, 0.0, Color32::from_rgb(0x25, 0x2a, 0x30));
    let mut child = ui.new_child(UiBuilder::new().max_rect(field.shrink2(vec2(8.0 * k, 0.0))));
    child.visuals_mut().override_text_color = Some(TEXT);
    let te = child.add_sized(
        vec2(field.width() - 16.0 * k, field.height()),
        egui::TextEdit::singleline(&mut st.url)
            .frame(egui::Frame::NONE)
            .font(pn.l.font(11.5, false))
            .hint_text("Address")
            .vertical_align(egui::Align::Center),
    );
    let go_clicked =
        flat_button(ui, pn, Id::new("ov-go"), pn.rect(w - 16.0 - 108.0, 58.0, 108.0, 32.0), "OPEN IN BROWSER", 8.6);
    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if (go_clicked || enter) && !st.url.trim().is_empty() {
        let u = st.url.trim().to_string();
        let u = if u.contains("://") { u } else { format!("https://{u}") };
        actions.push(OverlayAction::OpenUrl(u));
    }
    pn.text(16.0, 116.0, Align2::LEFT_CENTER, "QUICK LINKS", 9.5, TEXT_DIM, true);
    let a = data.app_id;
    let links = [
        ("Community Hub", format!("https://steamcommunity.com/app/{a}")),
        ("Store Page", format!("https://store.steampowered.com/app/{a}")),
        ("Guides", format!("https://steamcommunity.com/app/{a}/guides/")),
        ("Discussions", format!("https://steamcommunity.com/app/{a}/discussions/")),
        ("Workshop", format!("https://steamcommunity.com/app/{a}/workshop/")),
        ("Steam Support", "https://help.steampowered.com".to_string()),
    ];
    for (i, (label, url)) in links.iter().enumerate() {
        let (col, row) = (i % 2, i / 2);
        let r = pn.rect(16.0 + col as f32 * 264.0, 130.0 + row as f32 * 46.0, 256.0, 40.0);
        let resp = ui.interact(r, Id::new(("ov-quick", i)), Sense::click());
        pn.p.rect_filled(r, 0.0, if resp.hovered() { BTN_HOVER } else { ROW_BG });
        text(&pn.p, pn.l, r.min + vec2(12.0, 20.0) * k, Align2::LEFT_CENTER, label, 11.5, TEXT, true);
        if resp.clicked() {
            actions.push(OverlayAction::OpenUrl(url.clone()));
        }
    }
    pn.text(16.0, 290.0, Align2::LEFT_CENTER, "Pages open in your default browser.", 9.5, TEXT_DIM, false);
}

// ---- Settings ------------------------------------------------------------------------------------------------

fn settings(ui: &mut Ui, pn: &Pn, st: &mut OverlayState, actions: &mut Vec<OverlayAction>) {
    let k = pn.l.k;
    pn.text(16.0, 30.0, Align2::LEFT_CENTER, "Overlay Settings", 18.5, TEXT, true);
    let r = pn.rect(16.0, 62.0, 328.0, 44.0);
    let resp = ui.interact(r, Id::new("ov-set-perf"), Sense::click());
    pn.p.rect_filled(r, 0.0, if resp.hovered() { BTN_HOVER } else { ROW_BG });
    pn.text(28.0, 84.0, Align2::LEFT_CENTER, "In-game performance monitor", 11.0, TEXT, false);
    let tr = pn.rect(300.0, 75.0, 32.0, 18.0);
    pn.p.rect_filled(tr, 9.0 * k, if st.perf_hud { ACCENT } else { Color32::from_rgb(0x4b, 0x52, 0x5e) });
    let kx = if st.perf_hud { tr.max.x - 9.0 * k } else { tr.min.x + 9.0 * k };
    pn.p.circle_filled(pos2(kx, tr.center().y), 7.0 * k, Color32::WHITE);
    if resp.clicked() {
        st.perf_hud = !st.perf_hud;
        actions.push(OverlayAction::TogglePerfHud(st.perf_hud));
    }
    pn.text(16.0, 132.0, Align2::LEFT_CENTER, "SHORTCUTS", 9.5, TEXT_DIM, true);
    for (i, (key, what)) in
        [("Shift+Tab", "Open or close the overlay"), ("Esc", "Close the overlay"), ("F12", "Take a screenshot")]
            .iter()
            .enumerate()
    {
        let y = 156.0 + i as f32 * 28.0;
        pn.text(28.0, y, Align2::LEFT_CENTER, key, 10.5, TEXT, true);
        pn.text(120.0, y, Align2::LEFT_CENTER, what, 10.5, TEXT_DIM, false);
    }
}
