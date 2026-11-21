//! Design tokens. Every color, size and font the views use comes from here;
//! views never hard-code them. Values are approximations of the current
//! desktop client's library and get refined from reference screenshots.

use egui::{Color32, CornerRadius, FontFamily, FontId, Margin, Shadow, Stroke, Style, Visuals, vec2};

/// Shown to people (title, menus).
pub const APP_NAME: &str = "Kettle";
/// Lowercase id for the window class, folders and the keyring.
pub const APP_ID: &str = "kettle";

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
pub const BG_INPUT: Color32 = Color32::from_rgb(0x32, 0x35, 0x3c);
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

// Menus, drop-down lists and tooltips: a flat slate panel; the hovered row is inverted, like the official client.
pub const MENU_BG: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50);
pub const MENU_BORDER: Color32 = Color32::from_rgb(0x2c, 0x31, 0x3a);
pub const MENU_HOVER_BG: Color32 = Color32::from_rgb(0xdc, 0xde, 0xdf);
pub const MENU_HOVER_TEXT: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50);
pub const MENU_SEP: Color32 = Color32::from_rgb(0x23, 0x26, 0x2e);
/// The chosen entry of an open drop-down list.
pub const MENU_SELECTED: Color32 = Color32::from_rgb(0x54, 0x5d, 0x6b);

// Settings window.
pub const SETTINGS_NAV_BG: Color32 = Color32::from_rgb(0x18, 0x1b, 0x21);
pub const SETTINGS_BG: Color32 = Color32::from_rgb(0x23, 0x26, 0x2e);
pub const SETTINGS_SEP: Color32 = Color32::from_rgb(0x32, 0x36, 0x3f);
pub const TOGGLE_OFF: Color32 = Color32::from_rgb(0x52, 0x59, 0x66);
/// Flat buttons: grey, lighter on hover. The primary one is a blue gradient.
pub const BTN_FLAT: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50);
pub const BTN_FLAT_HOVER: Color32 = Color32::from_rgb(0x4f, 0x58, 0x66);
pub const BTN_PRIMARY_A: Color32 = Color32::from_rgb(0x1a, 0x9f, 0xff);
pub const BTN_PRIMARY_B: Color32 = Color32::from_rgb(0x14, 0x74, 0xd4);

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
/// Narrowest a menu gets, so short item lists do not look cramped.
pub const MENU_MIN_W: f32 = 200.0;
pub const SETTINGS_NAV_W: f32 = 220.0;
pub const SETTINGS_TITLE_H: f32 = 32.0;
pub const TOGGLE_W: f32 = 36.0;
pub const TOGGLE_H: f32 = 18.0;
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

/// The look of everything that opens over the page: menus, context menus, drop-down lists. Passed as the
/// style modifier of every popup (see `widgets::menu_button`, `widgets::combo`), so they all match.
/// Rows are full width: the panel has no side padding and the hover fill runs edge to edge.
pub fn menu_style(style: &mut Style) {
    style.spacing.button_padding = vec2(16.0, 3.5);
    style.spacing.item_spacing = vec2(0.0, 0.0);
    style.spacing.menu_margin = Margin::symmetric(0, 4);
    style.spacing.interact_size.y = 22.0;
    style.text_styles.insert(egui::TextStyle::Button, regular(13.5));
    style.text_styles.insert(egui::TextStyle::Body, regular(13.5));
    let v = &mut style.visuals;
    v.menu_corner_radius = CornerRadius::same(2);
    v.selection.bg_fill = MENU_SELECTED;
    v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.expansion = 0.0;
        w.bg_stroke = Stroke::NONE;
    }
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    for w in [&mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.weak_bg_fill = MENU_HOVER_BG;
        w.bg_fill = MENU_HOVER_BG;
        w.fg_stroke = Stroke::new(1.0, MENU_HOVER_TEXT);
    }
}

/// Apply the base look to egui's own widgets (scrollbars, text edits, menus, tooltips).
pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG_MAIN;
    // Popups and tooltips (dialogs and panels draw their own frames).
    v.window_fill = MENU_BG;
    v.window_stroke = Stroke::new(1.0, MENU_BORDER);
    v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(90) };
    v.menu_corner_radius = CornerRadius::same(2);
    v.extreme_bg_color = BG_INPUT;
    // No global text override: menus invert the text color on hover, which an override would forbid.
    v.override_text_color = None;
    v.hyperlink_color = ACCENT;
    // Text fields: dark fill, no border, blue outline while focused (egui draws the outline with this stroke).
    v.selection.bg_fill = ACCENT.linear_multiply(0.5);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_stroke = Stroke::NONE;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    // `bg_fill` is also the scrollbar handle: slate, lighter while hovered or dragged.
    v.widgets.inactive.bg_fill = BTN_FLAT;
    v.widgets.inactive.weak_bg_fill = BG_INPUT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.hovered.bg_fill = BTN_FLAT_HOVER;
    v.widgets.hovered.weak_bg_fill = BG_ROW_SELECTED;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.bg_fill = BTN_FLAT_HOVER;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.open.bg_stroke = Stroke::NONE;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = BUTTON_RADIUS;
    }
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        // Thin rounded bars that widen under the pointer; invisible until the area is scrolled or hovered.
        s.spacing.scroll =
            egui::style::ScrollStyle { bar_width: 9.0, floating_width: 3.0, ..egui::style::ScrollStyle::thin() };
        s.spacing.menu_margin = Margin::symmetric(8, 5);
        s.interaction.selectable_labels = false;
    });
}
