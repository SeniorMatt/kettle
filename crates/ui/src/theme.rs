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
/// Hairline around the window (we draw our own frame).
pub const WINDOW_BORDER: Color32 = Color32::from_rgb(0x2c, 0x31, 0x3b);
/// Close button hover.
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xe8, 0x11, 0x23);
/// Store discount box (dark green with light green text, like the store).
pub const DISCOUNT_BG: Color32 = Color32::from_rgb(0x4c, 0x6b, 0x22);
pub const DISCOUNT_TEXT: Color32 = Color32::from_rgb(0xbe, 0xee, 0x11);
/// Price tag background next to a discount box.
pub const PRICE_BG: Color32 = Color32::from_rgb(0x30, 0x3a, 0x44);
pub const ACCENT: Color32 = Color32::from_rgb(0x1a, 0x9f, 0xff);
/// Destructive buttons (uninstall) and error text.
pub const DANGER: Color32 = Color32::from_rgb(0xa8, 0x32, 0x32);
pub const ERROR_TEXT: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x5c);
/// Download progress for verifying (hashing) instead of fetching.
pub const VERIFY: Color32 = Color32::from_rgb(0x75, 0xb0, 0x22);
/// A card on a page (download rows, settings groups).
pub const BG_CARD: Color32 = Color32::from_rgb(0x23, 0x27, 0x30);

/// Play button gradient (left → right).
pub const PLAY_A: Color32 = Color32::from_rgb(0x75, 0xb0, 0x22);
pub const PLAY_B: Color32 = Color32::from_rgb(0x58, 0x8a, 0x1b);
/// Install button gradient.
pub const INSTALL_A: Color32 = Color32::from_rgb(0x47, 0xbf, 0xff);
pub const INSTALL_B: Color32 = Color32::from_rgb(0x1a, 0x44, 0xc2);

/// Friend names by state, as in the official friends list.
pub const FRIEND_INGAME: Color32 = Color32::from_rgb(0x90, 0xba, 0x3c);
pub const FRIEND_ONLINE: Color32 = Color32::from_rgb(0x57, 0xcb, 0xde);
pub const FRIEND_AWAY: Color32 = Color32::from_rgb(0x4a, 0x8f, 0x9f);
pub const FRIEND_OFFLINE: Color32 = Color32::from_rgb(0x7a, 0x80, 0x88);
/// Chat bubbles: ours and theirs.
pub const CHAT_MINE: Color32 = Color32::from_rgb(0x1f, 0x4b, 0x73);
pub const CHAT_THEIRS: Color32 = Color32::from_rgb(0x2f, 0x33, 0x3d);

// ---- sizes (points) -----------------------------------------------------

pub const MENU_BAR_H: f32 = 26.0;
pub const NAV_BAR_H: f32 = 52.0;
pub const BOTTOM_BAR_H: f32 = 40.0;
pub const SIDEBAR_W: f32 = 280.0;
pub const FRIENDS_W: f32 = 320.0;
pub const LIST_ROW_H: f32 = 26.0;
pub const CAPSULE_W: f32 = 150.0;
pub const CAPSULE_H: f32 = 225.0;
pub const GRID_GAP: f32 = 16.0;
pub const WINDOW_BUTTON_W: f32 = 44.0;
/// Invisible zone along each window edge that starts a resize.
pub const RESIZE_EDGE: f32 = 6.0;
pub const STORE_CARD_W: f32 = 230.0;
pub const STORE_CARD_H: f32 = 107.0 + 30.0;
pub const PAGE_SLIDE: f32 = 8.0;
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
