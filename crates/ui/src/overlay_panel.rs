//! The content of the in-game overlay, drawn by the helper process (`kettle --overlay`, see `crates/app`).
//!
//! It is a pure view of an [`OverlaySnapshot`] that the main app sends: a dimmed full screen with a top bar
//! (game, session time, clock, Friends / Settings / Return), a quick settings card and notes on the left, the
//! performance overlay (MangoHud) settings in the middle and the friends list with chats on the right.
//! It never does I/O: what the user does comes out as [`FromHelper`] messages for the app to act on.

use crate::{anim, fmt, theme};
use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Modifiers, Pos2, Rect, Response,
    RichText, ScrollArea, Sense, Ui, UiBuilder, vec2,
};
use egui_phosphor::regular as icon;
use sr_client::{
    AnimationMode, ChatRow, FromHelper, HUD_INSTALL_HINTS, HUD_INSTALL_URL, HudColor, HudFontSize, HudPosition,
    HudPreset, HudSettings, MAX_NOTE_CHARS, OverlaySnapshot, PersonaState,
};
use std::time::{Duration, Instant};

/// Height of every control, like the official overlay's big buttons.
const CTL_H: f32 = 40.0;
const MARGIN: f32 = 48.0;
const TOP_H: f32 = 72.0;
const LEFT_W: f32 = 460.0;
const FRIENDS_W: f32 = 380.0;
const MIN_MID_W: f32 = 360.0;
const FADE_SECS: f32 = 0.12;
const DIM_ALPHA: f32 = 0.84;
/// A snapshot's HUD values are ignored for this long after the user changed one (the answer is still on its way).
const HUD_HOLD: Duration = Duration::from_millis(1500);

/// What one frame produced.
#[derive(Default)]
pub struct PanelOutput {
    pub send: Vec<FromHelper>,
    /// The user wants the overlay closed (Escape, "Return to game").
    pub close: bool,
}

pub struct OverlayPanel {
    snap: Option<OverlaySnapshot>,
    snap_at: Instant,
    /// Egui time of the first frame since the overlay was shown (the fade starts there).
    since: Option<f64>,
    show_friends: bool,
    show_settings: bool,
    chat: Option<u64>,
    input: String,
    note: String,
    hud: HudSettings,
    hud_touched: Option<Instant>,
    /// The last preset that was not "off", for the quick on/off switch.
    last_on: HudPreset,
}

impl Default for OverlayPanel {
    fn default() -> Self {
        OverlayPanel {
            snap: None,
            snap_at: Instant::now(),
            since: None,
            show_friends: true,
            show_settings: false,
            chat: None,
            input: String::new(),
            note: String::new(),
            hud: HudSettings::default(),
            hud_touched: None,
            last_on: HudPreset::FpsOnly,
        }
    }
}

impl OverlayPanel {
    /// The overlay is shown now: start from `snap` and fade in.
    pub fn on_show(&mut self, snap: OverlaySnapshot) {
        self.since = None;
        self.chat = None;
        self.input.clear();
        self.note = snap.note.clone();
        self.hud = snap.hud.clone();
        self.hud_touched = None;
        if snap.hud.preset != HudPreset::Off {
            self.last_on = snap.hud.preset;
        }
        self.snap_at = Instant::now();
        self.snap = Some(snap);
    }

    /// New data while shown. Text the user is typing is never replaced.
    pub fn update(&mut self, snap: OverlaySnapshot) {
        if self.hud_touched.is_none_or(|t| t.elapsed() > HUD_HOLD) {
            self.hud = snap.hud.clone();
        }
        self.snap_at = Instant::now();
        self.snap = Some(snap);
    }

    /// Messages for what the user left unsaved (the note); call when hiding.
    pub fn flush(&mut self) -> Vec<FromHelper> {
        match &self.snap {
            Some(s) if s.note != self.note => {
                vec![FromHelper::SetNote { app: s.app, text: self.note.clone() }]
            }
            _ => vec![],
        }
    }

    /// Draw one frame (call from the helper window's `ui`).
    pub fn show(&mut self, ui: &mut Ui) -> PanelOutput {
        let mut out = PanelOutput::default();
        let Some(snap) = self.snap.take() else { return out };
        let ctx = ui.ctx().clone();
        anim::set_mode(&ctx, if snap.animations { AnimationMode::Full } else { AnimationMode::Off });
        let now = ctx.input(|i| i.time);
        let since = *self.since.get_or_insert(now);
        let fade = anim::since(&ctx, since, FADE_SECS);
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            out.close = true;
        }
        let frame = Frame::new().fill(Color32::from_rgba_unmultiplied(8, 10, 14, (255.0 * DIM_ALPHA * fade) as u8));
        egui::CentralPanel::default().frame(frame).show(ui, |ui| {
            ui.set_opacity(fade);
            self.layout(ui, &snap, &mut out);
        });
        // The clock and the session time change once a minute.
        let wall = fmt::now_unix();
        ctx.request_repaint_after(Duration::from_secs(60 - wall % 60) + Duration::from_millis(200));
        self.snap = Some(snap);
        out
    }

    fn layout(&mut self, ui: &mut Ui, snap: &OverlaySnapshot, out: &mut PanelOutput) {
        let full = ui.max_rect();
        let top = Rect::from_min_size(full.min + vec2(MARGIN, 0.0), vec2(full.width() - 2.0 * MARGIN, TOP_H));
        region(ui, top, |ui| self.top_bar(ui, snap, out));
        ui.painter().line_segment(
            [top.left_bottom(), top.right_bottom()],
            egui::Stroke::new(1.0, Color32::from_white_alpha(24)),
        );

        let (y0, y1) = (top.bottom() + 20.0, full.bottom() - MARGIN);
        let left =
            Rect::from_min_max(Pos2::new(full.left() + MARGIN, y0), Pos2::new(full.left() + MARGIN + LEFT_W, y1));
        let right =
            Rect::from_min_max(Pos2::new(full.right() - MARGIN - FRIENDS_W, y0), Pos2::new(full.right() - MARGIN, y1));
        let mid_right = if self.show_friends { right.left() - 20.0 } else { full.right() - MARGIN };
        let mid = Rect::from_min_max(Pos2::new(left.right() + 20.0, y0), Pos2::new(mid_right, y1));

        let (quick_h, gap) = (self.quick_height(snap), 16.0);
        let quick = Rect::from_min_size(left.min, vec2(left.width(), quick_h));
        let notes = Rect::from_min_max(Pos2::new(left.left(), quick.bottom() + gap), left.max);
        card(ui, quick, |ui| self.quick_settings(ui, snap, out));
        if notes.height() > 120.0 {
            card(ui, notes, |ui| self.notes(ui, snap, out));
        }
        if self.show_settings && mid.width() >= MIN_MID_W {
            card(ui, mid, |ui| self.hud_panel(ui, snap, out));
        }
        if self.show_friends {
            card(ui, right, |ui| self.friends_panel(ui, snap, out));
        }
    }

    // ---- top bar ---------------------------------------------------------------------------------------

    fn top_bar(&mut self, ui: &mut Ui, snap: &OverlaySnapshot, out: &mut PanelOutput) {
        let r = ui.max_rect();
        ui.painter().text(
            r.left_center(),
            Align2::LEFT_CENTER,
            format!("{}  {}", icon::GAME_CONTROLLER, snap.game),
            theme::bold(26.0),
            theme::TEXT,
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            if ctl(ui, &format!("{}  Return to game", icon::ARROW_U_UP_LEFT), true).clicked() {
                out.close = true;
            }
            if ctl(ui, &format!("{}  Settings", icon::GEAR), self.show_settings).clicked() {
                self.show_settings = !self.show_settings;
            }
            if ctl(ui, &format!("{}  Friends", icon::USERS), self.show_friends).clicked() {
                self.show_friends = !self.show_friends;
            }
            ui.add_space(12.0);
            let wall = fmt::now_unix();
            let clock = fmt::clock(wall, sr_client::local_utc_offset(wall));
            ui.label(RichText::new(format!("{} {clock}", icon::CLOCK)).font(theme::medium(18.0)).color(theme::TEXT));
            ui.add_space(8.0);
            let secs = snap.session_secs + self.snap_at.elapsed().as_secs();
            ui.label(
                RichText::new(format!("{} Session {}", icon::TIMER, crate::overlay::session_text(secs)))
                    .font(theme::medium(16.0))
                    .color(theme::TEXT_DIM),
            );
        });
    }

    // ---- quick settings --------------------------------------------------------------------------------

    fn quick_height(&self, snap: &OverlaySnapshot) -> f32 {
        let mut h = 16.0 * 2.0 + 20.0 + 8.0 + 2.0 * (CTL_H + 6.0) + 34.0 + CTL_H;
        if self.hud.preset != HudPreset::Off && !snap.hud_installed {
            h += 24.0;
        }
        h
    }

    fn quick_settings(&mut self, ui: &mut Ui, snap: &OverlaySnapshot, out: &mut PanelOutput) {
        section(ui, "QUICK SETTINGS");
        let mut overlay = snap.overlay_on;
        if row_toggle(ui, "Overlay (Shift+Tab)", &mut overlay) {
            out.send.push(FromHelper::SetOverlay { on: overlay });
        }
        let mut hud_on = self.hud.preset != HudPreset::Off;
        if row_toggle(ui, "Performance overlay (FPS)", &mut hud_on) {
            let mut h = self.hud.clone();
            h.preset = if hud_on { self.last_on } else { HudPreset::Off };
            self.set_hud(h, out);
        }
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
            for p in HudPreset::ALL {
                if ctl(ui, p.short(), self.hud.preset == p).clicked() {
                    let mut h = self.hud.clone();
                    h.preset = p;
                    self.set_hud(h, out);
                }
            }
        });
        if self.hud.preset != HudPreset::Off && !snap.hud_installed {
            ui.label(
                RichText::new("MangoHud is not installed; see Settings.")
                    .color(theme::ERROR_TEXT)
                    .font(theme::regular(14.0)),
            );
        }
    }

    fn set_hud(&mut self, hud: HudSettings, out: &mut PanelOutput) {
        if hud.preset != HudPreset::Off {
            self.last_on = hud.preset;
        }
        self.hud_touched = Some(Instant::now());
        self.hud = hud.clone();
        out.send.push(FromHelper::SetHud(hud));
    }

    // ---- notes -----------------------------------------------------------------------------------------

    fn notes(&mut self, ui: &mut Ui, snap: &OverlaySnapshot, out: &mut PanelOutput) {
        section(ui, "NOTES");
        ui.label(
            RichText::new("Saved for this game, on this computer.").color(theme::TEXT_DIM).font(theme::regular(13.0)),
        );
        ui.add_space(8.0);
        let h = (ui.available_height() - 4.0).max(60.0);
        let edit = egui::TextEdit::multiline(&mut self.note)
            .id(Id::new("overlay-note"))
            .hint_text("Write something for this game...")
            .font(theme::regular(15.0))
            .desired_width(f32::INFINITY)
            .char_limit(MAX_NOTE_CHARS)
            .margin(Margin::symmetric(10, 8));
        let resp = ScrollArea::vertical()
            .max_height(h)
            .id_salt("overlay-note-scroll")
            .show(ui, |ui| {
                ui.visuals_mut().text_cursor.blink = false;
                ui.add_sized(vec2(ui.available_width(), h - 4.0), edit)
            })
            .inner;
        if resp.lost_focus() && self.note != snap.note {
            out.send.push(FromHelper::SetNote { app: snap.app, text: self.note.clone() });
        }
    }

    // ---- performance overlay ---------------------------------------------------------------------------

    fn hud_panel(&mut self, ui: &mut Ui, snap: &OverlaySnapshot, out: &mut PanelOutput) {
        section(ui, "PERFORMANCE OVERLAY (MANGOHUD)");
        ScrollArea::vertical().id_salt("overlay-hud").auto_shrink([false, false]).show(ui, |ui| {
            if !snap.hud_installed {
                not_installed(ui);
                ui.add_space(10.0);
            }
            let mut h = self.hud.clone();
            label(ui, "Show");
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                for p in HudPreset::ALL {
                    if ctl(ui, p.label(), h.preset == p).clicked() {
                        h.preset = p;
                    }
                }
            });
            if h.preset == HudPreset::Custom {
                label(ui, "Items");
                let mut items = h.custom;
                ui.columns(2, |cols| {
                    for (i, (name, field)) in items.fields().into_iter().enumerate() {
                        row_toggle(&mut cols[i % 2], name, field);
                    }
                });
                h.custom = items;
            }
            label(ui, "Position");
            choices(ui, &HudPosition::ALL, h.position, |p| p.label(), |p| h.position = p);
            label(ui, "Text size");
            choices(ui, &HudFontSize::ALL, h.font, |p| p.label(), |p| h.font = p);
            label(ui, "Text colour");
            choices(ui, &HudColor::ALL, h.color, |p| p.label(), |p| h.color = p);
            label(ui, "Background opacity");
            ui.spacing_mut().slider_width = 260.0;
            ui.spacing_mut().interact_size.y = CTL_H - 8.0;
            ui.add(egui::Slider::new(&mut h.opacity, 0..=100).suffix("%"));
            label(ui, "Frame rate limit");
            let fixed = sr_client::FPS_LIMIT_CHOICES;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                for v in fixed {
                    let text = if v == 0 { "None".to_string() } else { v.to_string() };
                    if ctl(ui, &text, h.fps_limit == v).clicked() {
                        h.fps_limit = v;
                    }
                }
                let custom = !fixed.contains(&h.fps_limit);
                if ctl(ui, "Custom", custom).clicked() && !custom {
                    h.fps_limit = 90;
                }
                if custom {
                    ui.add(egui::DragValue::new(&mut h.fps_limit).range(1..=1000).suffix(" fps"));
                }
            });
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!(
                    "Show or hide it in the game with {}. Changes apply the next time a game starts.",
                    h.toggle_key
                ))
                .font(theme::regular(13.5))
                .color(theme::TEXT_DIM),
            );
            if h != self.hud {
                self.set_hud(h, out);
            }
        });
    }

    // ---- friends and chat ------------------------------------------------------------------------------

    fn friends_panel(&mut self, ui: &mut Ui, snap: &OverlaySnapshot, out: &mut PanelOutput) {
        if let Some(id) = self.chat {
            self.chat_view(ui, snap, id, out);
            return;
        }
        section(ui, "FRIENDS & CHAT");
        if !snap.friends_connected {
            ui.label(RichText::new("Not connected to Steam.").color(theme::TEXT_DIM).font(theme::regular(15.0)));
            return;
        }
        let state = |f: &sr_client::FriendRow| PersonaState::from_wire(f.state);
        let groups: [(&str, Vec<&sr_client::FriendRow>); 3] = [
            (
                "IN-GAME",
                snap.friends.iter().filter(|f| f.game.is_some() && state(f) != PersonaState::Offline).collect(),
            ),
            ("ONLINE", snap.friends.iter().filter(|f| f.game.is_none() && state(f) != PersonaState::Offline).collect()),
            ("OFFLINE", snap.friends.iter().filter(|f| state(f) == PersonaState::Offline).collect()),
        ];
        if snap.friends.is_empty() {
            ui.label(RichText::new("No friends to show.").color(theme::TEXT_DIM).font(theme::regular(15.0)));
        }
        let mut open = None;
        ScrollArea::vertical().id_salt("overlay-friends").auto_shrink([false, false]).show(ui, |ui| {
            for (title, rows) in &groups {
                if rows.is_empty() {
                    continue;
                }
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("{title} ({})", rows.len()))
                        .font(theme::bold(12.0))
                        .color(theme::TEXT_HEADING),
                );
                for f in rows {
                    let unread = snap.chats.iter().find(|c| c.id == f.id).map_or(0, |c| c.unread);
                    if friend_row(ui, f, unread).clicked() {
                        open = Some(f.id);
                    }
                }
            }
        });
        if let Some(id) = open {
            self.chat = Some(id);
            self.input.clear();
            out.send.push(FromHelper::OpenChat { id });
        }
    }

    fn chat_view(&mut self, ui: &mut Ui, snap: &OverlaySnapshot, id: u64, out: &mut PanelOutput) {
        let name = snap.friends.iter().find(|f| f.id == id).map_or_else(|| id.to_string(), |f| f.name.clone());
        ui.horizontal(|ui| {
            if ctl(ui, &format!("{}  Friends", icon::CARET_LEFT), false).clicked() {
                self.chat = None;
            }
            ui.label(RichText::new(name).font(theme::bold(18.0)).color(theme::TEXT));
        });
        ui.add_space(8.0);
        let empty = ChatRow { id, unread: 0, messages: vec![] };
        let chat = snap.chats.iter().find(|c| c.id == id).unwrap_or(&empty);
        let offset = sr_client::local_utc_offset(fmt::now_unix());
        let list_h = (ui.available_height() - CTL_H - 12.0).max(40.0);
        ScrollArea::vertical().id_salt(("overlay-chat", id)).max_height(list_h).stick_to_bottom(true).show(ui, |ui| {
            ui.set_min_height(list_h);
            if chat.messages.is_empty() {
                ui.label(RichText::new("No messages yet.").color(theme::TEXT_DIM).font(theme::regular(14.0)));
            }
            let max_w = ui.available_width() * 0.82;
            for m in &chat.messages {
                let align = if m.mine { Align::Max } else { Align::Min };
                ui.with_layout(Layout::top_down(align), |ui| {
                    let fill = if m.mine { theme::CHAT_MINE } else { theme::CHAT_THEIRS };
                    Frame::new()
                        .fill(fill)
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(10, 6))
                        .show(ui, |ui| {
                            ui.set_max_width(max_w);
                            ui.label(RichText::new(&m.text).font(theme::regular(15.0)).color(theme::TEXT));
                        });
                    let (time, color) = if m.failed {
                        (format!("{}  not sent", fmt::clock(m.time, offset)), theme::ERROR_TEXT)
                    } else {
                        (fmt::clock(m.time, offset), theme::TEXT_DIM)
                    };
                    ui.label(RichText::new(time).font(theme::regular(11.0)).color(color));
                });
            }
        });
        ui.add_space(6.0);
        let edit = egui::TextEdit::singleline(&mut self.input)
            .id(Id::new(("overlay-chat-input", id)))
            .hint_text("Message")
            .font(theme::regular(16.0))
            .char_limit(sr_client::MAX_TEXT)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(10, 10));
        let resp = ui.add(edit);
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            let text = self.input.trim().to_string();
            if !text.is_empty() {
                out.send.push(FromHelper::SendChat { id, text });
                self.input.clear();
            }
            resp.request_focus();
        }
    }
}

// ---- small widgets ---------------------------------------------------------------------------------------

/// Run `add` in a child UI limited to `rect`.
fn region(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui)) {
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add(&mut child);
}

/// A rounded dark panel with padding.
fn card(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui)) {
    ui.painter().rect_filled(rect, CornerRadius::same(4), theme::BG_CARD);
    region(ui, rect.shrink(16.0), add);
}

fn section(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::bold(13.0)).color(theme::TEXT_HEADING));
    ui.add_space(8.0);
}

fn label(ui: &mut Ui, text: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).font(theme::medium(14.0)).color(theme::TEXT_HEADING));
    ui.add_space(4.0);
}

/// A big flat button; `selected` paints it in the accent colour.
fn ctl(ui: &mut Ui, text: &str, selected: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::medium(16.0), Color32::WHITE);
    let (rect, resp) = ui.allocate_exact_size(vec2(galley.size().x + 32.0, CTL_H), Sense::click());
    let fill = if selected {
        theme::BTN_PRIMARY_B
    } else if resp.hovered() {
        theme::BTN_FLAT_HOVER
    } else {
        theme::BTN_FLAT
    };
    ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, Color32::WHITE);
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// A full-width row with a switch on the right. True when it was clicked (and `on` flipped).
fn row_toggle(ui: &mut Ui, text: &str, on: &mut bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), CTL_H), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, theme::BG_ROW_HOVER);
    }
    ui.painter().text(
        rect.left_center() + vec2(10.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        theme::regular(16.0),
        theme::TEXT,
    );
    let pill = Rect::from_center_size(Pos2::new(rect.right() - 10.0 - 22.0, rect.center().y), vec2(44.0, 24.0));
    ui.painter().rect_filled(pill, CornerRadius::same(12), if *on { theme::ACCENT } else { theme::TOGGLE_OFF });
    let knob_x = if *on { pill.right() - 12.0 } else { pill.left() + 12.0 };
    ui.painter().circle_filled(Pos2::new(knob_x, pill.center().y), 9.0, Color32::WHITE);
    let clicked = resp.on_hover_cursor(CursorIcon::PointingHand).clicked();
    if clicked {
        *on = !*on;
    }
    clicked
}

/// A wrapped row of exclusive choices.
fn choices<T: Copy + PartialEq>(
    ui: &mut Ui,
    all: &[T],
    current: T,
    name: impl Fn(T) -> &'static str,
    mut pick: impl FnMut(T),
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
        for &v in all {
            if ctl(ui, name(v), v == current).clicked() {
                pick(v);
            }
        }
    });
}

fn friend_row(ui: &mut Ui, f: &sr_client::FriendRow, unread: u32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, theme::BG_ROW_HOVER);
    }
    let st = PersonaState::from_wire(f.state);
    let (color, text) = match (&f.game, st) {
        (_, PersonaState::Offline) => (theme::FRIEND_OFFLINE, "Offline".to_string()),
        (Some(g), _) => (theme::FRIEND_INGAME, format!("In-Game: {g}")),
        (None, PersonaState::Online) => (theme::FRIEND_ONLINE, "Online".to_string()),
        (None, PersonaState::Busy) => (theme::FRIEND_AWAY, "Busy".to_string()),
        (None, PersonaState::Away | PersonaState::Snooze) => (theme::FRIEND_AWAY, "Away".to_string()),
        (None, PersonaState::LookingToTrade) => (theme::FRIEND_ONLINE, "Looking to trade".to_string()),
        (None, PersonaState::LookingToPlay) => (theme::FRIEND_ONLINE, "Looking to play".to_string()),
        (None, PersonaState::Invisible) => (theme::FRIEND_OFFLINE, "Offline".to_string()),
    };
    let p = ui.painter();
    p.circle_filled(Pos2::new(rect.left() + 16.0, rect.center().y), 6.0, color);
    p.text(
        Pos2::new(rect.left() + 32.0, rect.center().y - 9.0),
        Align2::LEFT_CENTER,
        &f.name,
        theme::medium(16.0),
        color,
    );
    p.text(
        Pos2::new(rect.left() + 32.0, rect.center().y + 11.0),
        Align2::LEFT_CENTER,
        text,
        theme::regular(13.0),
        theme::TEXT_DIM,
    );
    if unread > 0 {
        p.text(
            Pos2::new(rect.right() - 12.0, rect.center().y),
            Align2::RIGHT_CENTER,
            format!("{} {unread}", icon::CHAT_CIRCLE),
            theme::medium(14.0),
            theme::ACCENT,
        );
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

/// The "install MangoHud" hint.
fn not_installed(ui: &mut Ui) {
    Frame::new().fill(theme::BG_INPUT).corner_radius(CornerRadius::same(4)).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("MangoHud is not installed").font(theme::bold(16.0)).color(theme::ERROR_TEXT));
        ui.label(
            RichText::new("It draws the FPS counter inside the game. Nothing else here depends on it. Install it, then start the game again:")
                .font(theme::regular(14.0))
                .color(theme::TEXT_DIM),
        );
        for (distro, cmd) in HUD_INSTALL_HINTS {
            ui.label(RichText::new(format!("{distro}:  {cmd}")).font(theme::regular(14.0)).color(theme::TEXT));
        }
        ui.label(RichText::new(HUD_INSTALL_URL).font(theme::regular(13.0)).color(theme::ACCENT));
    });
}
