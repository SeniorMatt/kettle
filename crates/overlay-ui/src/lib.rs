//! The in-game overlay's look (Shift+Tab), drawn with egui inside the game's own frames by the overlay hook
//! library. This crate only draws; it never talks to Steam or Kettle. The hook fills [`OverlayData`] from what
//! Kettle sends it and carries out the [`OverlayAction`]s.
//!
//! The layout follows Steam's 2023+ overlay: the game dimmed behind, clock and session at the top left, the
//! game's name in the middle, "Back to Game (Shift+Tab)" with a close button at the top right, free-standing
//! panels over the game (Game Overview, Achievements, Notes, Friends, ...) and the row of tool buttons at the
//! bottom. All sizes are measured in a 2000 x 1125 reference canvas and scaled with the window height.

mod icons;
mod panels;

use egui::{Align2, Color32, FontFamily, FontId, Id, Pos2, Rect, Sense, Vec2, pos2, vec2};

/// Everything the overlay shows. Filled by the hook from Kettle's messages; all fields may be empty.
#[derive(Debug, Clone, Default)]
pub struct OverlayData {
    pub app_id: u32,
    pub game_name: String,
    /// Minutes played in this session and in total.
    pub session_minutes: u32,
    pub total_minutes: u32,
    /// Local time, already formatted ("21:34").
    pub clock: String,
    /// Local date, already formatted ("Thu, June 8, 2023"). May be empty.
    pub date: String,
    pub account_name: String,
    /// The account's picture as square RGBA pixels, if loaded.
    pub account_avatar: Option<(usize, Vec<u8>)>,
    pub friends: Vec<OverlayFriend>,
    pub achievements: Option<OverlayAchievements>,
    pub perf: Option<OverlayPerf>,
}

#[derive(Debug, Clone, Default)]
pub struct OverlayFriend {
    pub steam_id: u64,
    pub name: String,
    /// "Online", "Away", or the game they are playing.
    pub status: String,
    pub in_game: bool,
    pub online: bool,
    /// Avatar as RGBA pixels (square), if loaded.
    pub avatar: Option<(usize, Vec<u8>)>,
}

#[derive(Debug, Clone, Default)]
pub struct OverlayAchievements {
    pub unlocked: u32,
    pub total: u32,
    /// Most recent first: (name, description, unlock time as unix seconds).
    pub recent: Vec<(String, String, u64)>,
}

#[derive(Debug, Clone, Default)]
pub struct OverlayPerf {
    pub fps: f32,
    pub frame_ms: f32,
}

/// What the user did in the overlay; the hook carries these out.
#[derive(Debug, Clone, PartialEq)]
pub enum OverlayAction {
    /// Close the overlay and give input back to the game.
    Close,
    OpenUrl(String),
    OpenChat(u64),
    TakeScreenshot,
    TogglePerfHud(bool),
}

/// The panels that can be open over the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Overview = 0,
    Achievements = 1,
    Notes = 2,
    Guides = 3,
    Friends = 4,
    Browser = 5,
    Settings = 6,
}
pub const PANEL_COUNT: usize = 7;

/// The overlay's own view state (open panels, scroll, text fields). Kept by the hook between frames.
#[derive(Debug, Default)]
pub struct OverlayState {
    pub perf_hud: bool,
    /// Which panels are open, indexed by [`Panel`]. On the first frame the usual three open by themselves.
    pub open: [bool; PANEL_COUNT],
    /// Set once the default panels were opened; set it yourself to start with everything closed.
    pub initialised: bool,
    /// Where the user dragged each panel to, relative to its home position (reference units).
    pub offsets: [Vec2; PANEL_COUNT],
    /// Scroll position of the list in each panel (points).
    pub scroll: [f32; PANEL_COUNT],
    pub notes: String,
    pub url: String,
    /// Selected tab of the Achievements panel (0 in progress, 1 mine, 2 global).
    pub ach_tab: u8,
}

impl OverlayState {
    pub fn is_open(&self, p: Panel) -> bool {
        self.open[p as usize]
    }
    pub fn toggle(&mut self, p: Panel) {
        self.open[p as usize] = !self.open[p as usize];
    }
}

// ---- colours (measured from Steam's overlay screenshots) ----------------------------------------------------
pub(crate) const PANEL_BG: Color32 = Color32::from_rgb(0x1f, 0x23, 0x2e);
pub(crate) const PANEL_BG2: Color32 = Color32::from_rgb(0x1c, 0x20, 0x29);
pub(crate) const ROW_BG: Color32 = Color32::from_rgb(0x23, 0x26, 0x2f);
pub(crate) const BAR_BG: Color32 = Color32::from_rgb(0x3d, 0x43, 0x4f);
pub(crate) const BTN_BG: Color32 = Color32::from_rgb(0x23, 0x26, 0x2f);
pub(crate) const BTN_HOVER: Color32 = Color32::from_rgb(0x32, 0x37, 0x42);
pub(crate) const BTN_ACTIVE: Color32 = Color32::from_rgb(0x1a, 0x88, 0xdd);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(0x1a, 0x9f, 0xff);
pub(crate) const TEXT: Color32 = Color32::from_rgb(0xee, 0xf0, 0xf2);
pub(crate) const TEXT_DIM: Color32 = Color32::from_rgb(0x8f, 0x98, 0xa0);
pub(crate) const ONLINE: Color32 = Color32::from_rgb(0x53, 0xb4, 0xea);
pub(crate) const ONLINE_DIM: Color32 = Color32::from_rgb(0x3f, 0x89, 0xb5);
pub(crate) const INGAME: Color32 = Color32::from_rgb(0x90, 0xba, 0x3c);
pub(crate) const INGAME_DIM: Color32 = Color32::from_rgb(0x6a, 0x8a, 0x2c);

const BOLD: &str = "overlay-bold";
const MEDIUM: &str = "overlay-medium";

/// Install the fonts the overlay needs (Figtree, as in Kettle's own window). [`show`] does this by itself on
/// the first frame; call it earlier only if you want the fonts ready before the overlay opens.
pub fn install_fonts(ctx: &egui::Context) {
    use egui::{FontData, FontDefinitions};
    use std::sync::Arc;
    let mut fonts = FontDefinitions::default();
    let add = |f: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "figtree", include_bytes!("../../ui/assets/fonts/Figtree-Regular.ttf"));
    add(&mut fonts, "figtree-medium", include_bytes!("../../ui/assets/fonts/Figtree-Medium.ttf"));
    add(&mut fonts, "figtree-bold", include_bytes!("../../ui/assets/fonts/Figtree-Bold.ttf"));
    add(&mut fonts, "inter", include_bytes!("../../ui/assets/fonts/Inter-Regular.ttf"));
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |first: &str| {
        let mut v = vec![first.to_string(), "inter".to_string()];
        v.extend(defaults.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Proportional, chain("figtree"));
    fonts.families.insert(FontFamily::Name(MEDIUM.into()), chain("figtree-medium"));
    fonts.families.insert(FontFamily::Name(BOLD.into()), chain("figtree-bold"));
    ctx.set_fonts(fonts);
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| d.insert_temp(Id::new("kettle-overlay-fonts"), pass));
}

/// The bold family, once the fonts are in effect (they apply from the pass after `set_fonts`).
fn bold_family(ctx: &egui::Context) -> FontFamily {
    let at = ctx.data(|d| d.get_temp::<u64>(Id::new("kettle-overlay-fonts")));
    match at {
        None => {
            install_fonts(ctx);
            FontFamily::Proportional
        }
        Some(n) if ctx.cumulative_pass_nr() > n => FontFamily::Name(BOLD.into()),
        Some(_) => FontFamily::Proportional,
    }
}

/// Reference-canvas to screen mapping: everything is laid out in a 2000 x 1125 canvas, scaled by the window
/// height (not below 0.75 and never wider than the window), centred horizontally.
#[derive(Clone)]
pub(crate) struct Layout {
    pub k: f32,
    pub screen: Rect,
    pub ox: f32,
    pub bold: FontFamily,
}

impl Layout {
    fn new(ctx: &egui::Context) -> Self {
        let screen = ctx.content_rect();
        let k = (screen.height() / 1125.0).max(0.75).min(screen.width() / 1800.0).max(0.3);
        let ox = screen.min.x + (screen.width() - 2000.0 * k) / 2.0;
        Layout { k, screen, ox, bold: bold_family(ctx) }
    }
    /// A canvas point from the top left.
    pub fn pt(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.ox + x * self.k, self.screen.min.y + y * self.k)
    }
    pub fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.pt(x, y), vec2(w, h) * self.k)
    }
    pub fn font(&self, size: f32, bold: bool) -> FontId {
        FontId::new(
            size * if size < 13.0 { 1.15 } else { 1.0 } * self.k,
            if bold { self.bold.clone() } else { FontFamily::Proportional },
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn text(
    p: &egui::Painter,
    l: &Layout,
    pos: Pos2,
    a: Align2,
    s: &str,
    size: f32,
    c: Color32,
    bold: bool,
) -> Rect {
    p.text(pos, a, s, l.font(size, bold), c)
}

/// Draw the whole overlay for one frame (the dimmed backdrop, top and bottom bars, open panels).
pub fn show(ctx: &egui::Context, data: &OverlayData, state: &mut OverlayState) -> Vec<OverlayAction> {
    let mut actions = Vec::new();
    let l = Layout::new(ctx);
    if !state.initialised {
        state.initialised = true;
        if !state.open.iter().any(|o| *o) {
            state.open[Panel::Overview as usize] = true;
            state.open[Panel::Achievements as usize] = true;
            state.open[Panel::Friends as usize] = true;
        }
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(OverlayAction::Close);
    }

    // The game, dimmed. Nothing under it gets clicks.
    egui::Area::new(Id::new("kettle-ov-backdrop"))
        .order(egui::Order::Background)
        .fade_in(false)
        .fixed_pos(l.screen.min)
        .show(ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(l.screen.size(), Sense::click_and_drag());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha(222));
        });

    top_bar(ctx, &l, data, &mut actions);
    panels::all(ctx, &l, data, state, &mut actions);
    toolbar(ctx, &l, state, &mut actions);
    if let (true, Some(perf)) = (state.perf_hud, &data.perf) {
        show_hud(ctx, perf);
    }
    actions
}

fn top_bar(ctx: &egui::Context, l: &Layout, data: &OverlayData, actions: &mut Vec<OverlayAction>) {
    egui::Area::new(Id::new("kettle-ov-top")).order(egui::Order::Middle).fade_in(false).fixed_pos(l.screen.min).show(
        ctx,
        |ui| {
            let p = ui.painter().clone();
            if !data.clock.is_empty() {
                text(&p, l, l.pt(250.0, 27.0), Align2::LEFT_CENTER, &data.clock, 18.0, TEXT, true);
            }
            if !data.date.is_empty() {
                text(&p, l, l.pt(250.0, 50.0), Align2::LEFT_CENTER, &data.date, 10.5, TEXT, false);
            }
            let session = if data.session_minutes == 1 {
                "1 minute - this session".to_string()
            } else {
                format!("{} minutes - this session", data.session_minutes)
            };
            text(&p, l, l.pt(250.0, 66.0), Align2::LEFT_CENTER, &session, 10.5, TEXT, false);

            text(&p, l, l.pt(1000.0, 84.0), Align2::CENTER_CENTER, &data.game_name, 34.0, TEXT, true);

            text(&p, l, l.pt(1699.0, 27.0), Align2::RIGHT_CENTER, "Back to Game", 17.0, TEXT, true);
            text(&p, l, l.pt(1699.0, 50.0), Align2::RIGHT_CENTER, "(Shift+Tab)", 10.5, TEXT_DIM, false);
            let btn = l.rect(1711.0, 17.0, 38.0, 40.0);
            let r = ui.interact(btn, Id::new("kettle-ov-close"), Sense::click());
            p.rect_filled(btn, 0.0, if r.hovered() { BTN_HOVER } else { BTN_BG });
            icons::cross(&p, Rect::from_center_size(btn.center(), Vec2::splat(13.0 * l.k)), TEXT, 2.0 * l.k);
            if r.clicked() {
                actions.push(OverlayAction::Close);
            }
        },
    );
}

/// The row of tool buttons at the bottom.
fn toolbar(ctx: &egui::Context, l: &Layout, state: &mut OverlayState, actions: &mut Vec<OverlayAction>) {
    #[derive(Clone, Copy)]
    enum Tool {
        P(Panel),
        Shot,
        Perf,
    }
    let tools: [(Tool, &str); 9] = [
        (Tool::P(Panel::Overview), "Game Overview"),
        (Tool::P(Panel::Achievements), "Achievements"),
        (Tool::P(Panel::Notes), "Notes"),
        (Tool::P(Panel::Guides), "Guides"),
        (Tool::Shot, "Take Screenshot"),
        (Tool::P(Panel::Friends), "Friends & Chat"),
        (Tool::P(Panel::Browser), "Web Browser"),
        (Tool::P(Panel::Settings), "Settings"),
        (Tool::Perf, "Performance Monitor"),
    ];
    let y = 1048.0;
    let bottom = l.screen.max.y - 1125.0 * l.k + y * l.k; // anchor to the window's bottom edge
    let top_y = bottom;
    egui::Area::new(Id::new("kettle-ov-toolbar"))
        .order(egui::Order::Middle)
        .fade_in(false)
        .fixed_pos(l.screen.min)
        .show(ctx, |ui| {
            let p = ui.painter().clone();
            let n = tools.len() as f32;
            let x0 = 1000.0 - ((n - 1.0) * 43.0 + 36.0) / 2.0;
            for (i, (tool, label)) in tools.iter().enumerate() {
                let min = pos2(l.pt(x0 + i as f32 * 43.0, 0.0).x, top_y);
                let r = Rect::from_min_size(min, Vec2::splat(36.0 * l.k));
                let resp = ui.interact(r, Id::new(("kettle-ov-tool", i)), Sense::click());
                let active = match tool {
                    Tool::P(pn) => state.is_open(*pn),
                    Tool::Shot => false,
                    Tool::Perf => state.perf_hud,
                };
                let bg = if active {
                    BTN_ACTIVE
                } else if resp.hovered() {
                    BTN_HOVER
                } else {
                    BTN_BG
                };
                p.rect_filled(r, 2.0 * l.k, bg);
                let ir = Rect::from_center_size(r.center(), Vec2::splat(19.0 * l.k));
                let c = if active || resp.hovered() { Color32::WHITE } else { Color32::from_rgb(0xdc, 0xde, 0xdf) };
                match i {
                    0 => icons::info(&p, ir, c, bg),
                    1 => icons::medal(&p, ir, c, bg),
                    2 => icons::pencil(&p, ir, c),
                    3 => icons::book(&p, ir, c, bg),
                    4 => icons::screenshot(&p, ir, c),
                    5 => icons::people(&p, ir, c),
                    6 => icons::compass(&p, ir, c),
                    7 => icons::gear(&p, ir, c, bg),
                    _ => icons::gauge(&p, ir, c),
                }
                if resp.hovered() {
                    let tip_pos = pos2(r.center().x, r.min.y - 9.0 * l.k);
                    let g = p.layout_no_wrap(label.to_string(), l.font(11.5, false), TEXT);
                    let box_r = Rect::from_center_size(
                        pos2(tip_pos.x, tip_pos.y - g.size().y / 2.0),
                        g.size() + vec2(18.0, 10.0) * l.k,
                    );
                    p.rect_filled(box_r, 2.0 * l.k, Color32::from_rgb(0x3d, 0x44, 0x50));
                    p.galley(box_r.center() - g.size() / 2.0, g, TEXT);
                }
                if resp.clicked() {
                    match tool {
                        Tool::P(pn) => state.toggle(*pn),
                        Tool::Shot => actions.push(OverlayAction::TakeScreenshot),
                        Tool::Perf => {
                            state.perf_hud = !state.perf_hud;
                            actions.push(OverlayAction::TogglePerfHud(state.perf_hud));
                        }
                    }
                }
            }
        });
}

/// Draw only the small performance HUD (shown while the overlay is closed and the HUD is on).
pub fn show_hud(ctx: &egui::Context, perf: &OverlayPerf) {
    let l = Layout::new(ctx);
    egui::Area::new(Id::new("kettle-hud"))
        .order(egui::Order::Foreground)
        .fade_in(false)
        .interactable(false)
        .fixed_pos(l.pt(12.0, 12.0))
        .show(ctx, |ui| {
            let p = ui.painter().clone();
            let w = 96.0;
            let h = 56.0;
            let (r, _) = ui.allocate_exact_size(vec2(w, h) * l.k, Sense::hover());
            p.rect_filled(r, 3.0 * l.k, Color32::from_black_alpha(150));
            let num = format!("{:.0}", perf.fps);
            let nr = text(
                &p,
                &l,
                pos2(r.min.x + 10.0 * l.k, r.min.y + 4.0 * l.k),
                Align2::LEFT_TOP,
                &num,
                24.0,
                Color32::WHITE,
                true,
            );
            text(
                &p,
                &l,
                pos2(nr.max.x + 5.0 * l.k, r.min.y + 17.0 * l.k),
                Align2::LEFT_TOP,
                "FPS",
                11.0,
                TEXT_DIM,
                true,
            );
            text(
                &p,
                &l,
                pos2(r.min.x + 10.0 * l.k, r.max.y - 5.0 * l.k),
                Align2::LEFT_BOTTOM,
                &format!("{:.1} ms", perf.frame_ms),
                11.0,
                TEXT_DIM,
                false,
            );
        });
}

#[cfg(test)]
mod tests;
