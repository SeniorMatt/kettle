//! Design tokens. Every color, size and font the views use comes from here;
//! views never hard-code them. Values are approximations of the current
//! desktop client's library and get refined from reference screenshots.

use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, Visuals};

pub const APP_NAME: &str = "steam-rust";

// ---- colors -------------------------------------------------------------

/// Title bar, menu row and nav tabs.
pub const BG_TOP: Color32 = Color32::from_rgb(0x17, 0x1a, 0x21);
/// Library content area.
pub const BG_MAIN: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x25);
/// Left game list.
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x23, 0x26, 0x2e);
/// Bottom bar (downloads, friends).
pub const BG_BOTTOM: Color32 = Color32::from_rgb(0x1e, 0x21, 0x28);
/// Text inputs (search).
pub const BG_INPUT: Color32 = Color32::from_rgb(0x2f, 0x33, 0x3d);
/// Hovered and selected rows.
pub const BG_ROW_HOVER: Color32 = Color32::from_rgb(0x32, 0x36, 0x40);
pub const BG_ROW_SELECTED: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50);
/// Placeholder behind a cover that hasn't loaded.
pub const BG_CAPSULE: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x37);
/// Bar under the hero image on a game page.
pub const BG_ACTION_BAR: Color32 = Color32::from_rgb(0x20, 0x24, 0x2c);

pub const TEXT: Color32 = Color32::from_rgb(0xdc, 0xde, 0xdf);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8b, 0x92, 0x9a);
/// Not-installed games in the list.
pub const TEXT_UNINSTALLED: Color32 = Color32::from_rgb(0x6e, 0x75, 0x7e);
pub const TEXT_HEADING: Color32 = Color32::from_rgb(0xb8, 0xbc, 0xbf);
pub const ACCENT: Color32 = Color32::from_rgb(0x1a, 0x9f, 0xff);

/// Play button gradient (left → right).
pub const PLAY_A: Color32 = Color32::from_rgb(0x75, 0xb0, 0x22);
pub const PLAY_B: Color32 = Color32::from_rgb(0x58, 0x8a, 0x1b);
/// Install button gradient.
pub const INSTALL_A: Color32 = Color32::from_rgb(0x47, 0xbf, 0xff);
pub const INSTALL_B: Color32 = Color32::from_rgb(0x1a, 0x44, 0xc2);

// ---- sizes (points) -----------------------------------------------------

pub const MENU_BAR_H: f32 = 26.0;
pub const NAV_BAR_H: f32 = 52.0;
pub const BOTTOM_BAR_H: f32 = 40.0;
pub const SIDEBAR_W: f32 = 280.0;
pub const LIST_ROW_H: f32 = 26.0;
pub const CAPSULE_W: f32 = 150.0;
pub const CAPSULE_H: f32 = 225.0;
pub const GRID_GAP: f32 = 16.0;
pub const PAGE_PAD: f32 = 24.0;
pub const HERO_ASPECT: f32 = 1240.0 / 3840.0;
pub const RADIUS: CornerRadius = CornerRadius::same(2);
pub const BUTTON_RADIUS: CornerRadius = CornerRadius::same(3);

// ---- fonts --------------------------------------------------------------

pub const FAMILY_MEDIUM: &str = "medium";
pub const FAMILY_BOLD: &str = "bold";

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FAMILY_MEDIUM.into()))
}

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FAMILY_BOLD.into()))
}

/// Apply the base look to egui's own widgets (scrollbars, text edits, menus).
pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG_MAIN;
    v.window_fill = BG_SIDEBAR;
    v.extreme_bg_color = BG_INPUT;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.linear_multiply(0.5);
    v.widgets.noninteractive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.weak_bg_fill = BG_INPUT;
    v.widgets.inactive.corner_radius = BUTTON_RADIUS;
    v.widgets.hovered.weak_bg_fill = BG_ROW_SELECTED;
    v.widgets.hovered.corner_radius = BUTTON_RADIUS;
    v.widgets.active.corner_radius = BUTTON_RADIUS;
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.spacing.scroll = egui::style::ScrollStyle::thin();
        s.interaction.selectable_labels = false;
    });
}
