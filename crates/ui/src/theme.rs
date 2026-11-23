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
pub const BG_TOP: Color32 = Color32::from_rgb(0x17, 0x1d, 0x24); // title bar, nav row, bottom bar (measured)
/// Library content area.
pub const BG_MAIN: Color32 = Color32::from_rgb(0x24, 0x28, 0x30); // page and sidebar list (measured)
/// Left game list.
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x24, 0x28, 0x30); // sidebar list (measured)
/// Bottom bar (downloads, friends).
pub const BG_BOTTOM: Color32 = Color32::from_rgb(0x17, 0x1d, 0x24); // bottom bar (measured)
/// Text inputs (search).
pub const BG_INPUT: Color32 = Color32::from_rgb(0x25, 0x26, 0x2c); // sidebar search row (measured)
/// Hovered and selected rows.
pub const BG_ROW_HOVER: Color32 = Color32::from_rgb(0x31, 0x3a, 0x4a); // hover row: selection colour at half strength (derived)
pub const BG_ROW_SELECTED: Color32 = Color32::from_rgb(0x3e, 0x4f, 0x69); // selected list row (measured)
/// Placeholder behind a cover that hasn't loaded.
pub const BG_CAPSULE: Color32 = Color32::from_rgb(0x2b, 0x31, 0x38); // measured
/// Bar under the hero image on a game page.
pub const BG_ACTION_BAR: Color32 = Color32::from_rgb(0x28, 0x2b, 0x31); // game page play bar (measured)

pub const TEXT: Color32 = Color32::from_rgb(0xde, 0xe4, 0xe9); // nav tabs (measured)
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8b, 0x92, 0x9a); // menu text (measured)
/// Not-installed games in the list.
pub const TEXT_UNINSTALLED: Color32 = Color32::from_rgb(0x7d, 0x80, 0x87); // measured
pub const TEXT_HEADING: Color32 = Color32::from_rgb(0xa1, 0xa0, 0xa3); // stat labels (measured)
/// Hairline around the window (we draw our own frame).
pub const WINDOW_BORDER: Color32 = Color32::from_rgb(0x19, 0x1c, 0x24); // window edge line (measured)
/// Close button hover.
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xe8, 0x11, 0x23); // estimate
/// Store discount box (dark green with light green text, like the store).
pub const DISCOUNT_BG: Color32 = Color32::from_rgb(0x4c, 0x6b, 0x22); // estimate
pub const DISCOUNT_TEXT: Color32 = Color32::from_rgb(0xbe, 0xee, 0x11); // estimate
/// Price tag background next to a discount box.
pub const PRICE_BG: Color32 = Color32::from_rgb(0x30, 0x3a, 0x44); // estimate
/// "Running" label in the game list.
pub const RUNNING: Color32 = Color32::from_rgb(0x82, 0xc1, 0x2a); // running-game icon (measured)
pub const ACCENT: Color32 = Color32::from_rgb(0x1a, 0x9f, 0xff); // selected nav tab, underline (measured #269aee-#2ba1f6)
/// Destructive buttons (uninstall) and error text.
pub const DANGER: Color32 = Color32::from_rgb(0xa8, 0x32, 0x32); // estimate
pub const ERROR_TEXT: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x5c); // estimate
/// Download progress for verifying (hashing) instead of fetching.
pub const VERIFY: Color32 = Color32::from_rgb(0x75, 0xb0, 0x22); // estimate
/// A card on a page (download rows, settings groups).
pub const BG_CARD: Color32 = Color32::from_rgb(0x2e, 0x33, 0x3b); // tab box and cards (measured)

// Menus, drop-down lists and tooltips: a flat slate panel; the hovered row is inverted, like the official client.
pub const MENU_BG: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50); // palette
pub const MENU_BORDER: Color32 = Color32::from_rgb(0x2c, 0x31, 0x3a); // estimate
pub const MENU_HOVER_BG: Color32 = Color32::from_rgb(0xdc, 0xde, 0xdf); // contextMenuItem:hover
pub const MENU_HOVER_TEXT: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50); // contextMenuItem:hover color
pub const MENU_SEP: Color32 = Color32::from_rgb(0x26, 0x27, 0x27); // contextMenuItemHR 1px #262727
/// The chosen entry of an open drop-down list.
pub const MENU_SELECTED: Color32 = Color32::from_rgb(0x54, 0x5d, 0x6b); // estimate

// ---- Steam's dialog controls -------------------------------------------------------------------------
// Measured from the official client's stylesheets (numbers only): DialogButton, DialogInput, DialogDropDown,
// DialogCheckbox, DialogSlider, DialogToggleField, DialogControlsSectionHeader, ModalPosition.

// Settings window (DialogContent: #25282e; the page list on the left is a darker column).
pub const SETTINGS_NAV_BG: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x24); // estimate
pub const SETTINGS_BG: Color32 = Color32::from_rgb(0x25, 0x28, 0x2e);
/// DialogControlsSectionHeader border-bottom.
pub const SETTINGS_SEP: Color32 = Color32::from_rgb(0x23, 0x26, 0x2e);
/// A hovered category in the page list.
pub const SETTINGS_NAV_HOVER: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x37); // estimate

// DialogButton: bg #3d4450, text #dfe3e6 14px, line-height 32px, radius 2px, fades in 0.2s.
pub const BTN_FLAT: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50);
/// DialogButton:hover (text turns white).
pub const BTN_FLAT_HOVER: Color32 = Color32::from_rgb(0x46, 0x4d, 0x58);
/// DialogButton:enabled:active.
pub const BTN_FLAT_ACTIVE: Color32 = Color32::from_rgb(0x39, 0x3f, 0x49);
pub const BTN_TEXT: Color32 = Color32::from_rgb(0xdf, 0xe3, 0xe6);
/// DialogButton:disabled: rgba(61,67,77,.35) with text rgb(70,77,88).
pub const BTN_DISABLED_BG: Color32 = Color32::from_rgba_premultiplied(21, 23, 27, 89);
pub const BTN_DISABLED_FG: Color32 = Color32::from_rgb(70, 77, 88);
pub const BTN_H: f32 = 32.0;
pub const BTN_PAD_X: f32 = 16.0;
pub const BTN_FONT: f32 = 14.0;
/// `.GenericConfirmDialog .DialogButton` min-width.
pub const BTN_MIN_W_DIALOG: f32 = 120.0;
/// Blue button (the announcements button, Install, dialog primary): flat, measured #199aff.
pub const BTN_PRIMARY_A: Color32 = Color32::from_rgb(0x19, 0x9a, 0xff);
pub const BTN_PRIMARY_B: Color32 = Color32::from_rgb(0x19, 0x9a, 0xff);
/// Destructive button: flat dark red.
pub const BTN_DANGER_A: Color32 = Color32::from_rgb(0xa8, 0x3a, 0x3a);
pub const BTN_DANGER_B: Color32 = Color32::from_rgb(0xa8, 0x3a, 0x3a);

// _DialogInputContainer / DialogInput: rgba(59,63,72,.5) over the dialog, 3px radius, 14px text.
pub const INPUT_BG: Color32 = Color32::from_rgb(0x30, 0x34, 0x3b); // estimate
pub const INPUT_BG_HOVER: Color32 = Color32::from_rgb(0x38, 0x3d, 0x45); // estimate
pub const INPUT_RADIUS: CornerRadius = CornerRadius::same(3);
/// `.DesktopUI ... .DialogInput` padding.
pub const INPUT_PAD: f32 = 8.0;
/// DialogDropDown:hover.
pub const DROPDOWN_HOVER: Color32 = Color32::from_rgb(0x46, 0x4d, 0x58);
/// DialogDropDown_Arrow fill.
pub const DROPDOWN_ARROW: Color32 = Color32::from_rgb(0x96, 0x96, 0x96);

// DialogToggleField_Option: active #2d73ff, "off" #5f6875.
pub const TOGGLE_ON: Color32 = Color32::from_rgb(0x2d, 0x73, 0xff);
pub const TOGGLE_OFF: Color32 = Color32::from_rgb(0x5f, 0x68, 0x75);

// DialogCheckbox: rgba(0,0,0,.267), 2px radius.
pub const CHECK_BG: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 68);
pub const CHECK_BG_HOVER: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 110); // estimate
pub const CHECK_SIZE: f32 = 18.0;

// DialogSlider: dark track, flat blue value, light grabber.
pub const SLIDER_TRACK: Color32 = Color32::from_rgb(0x20, 0x22, 0x26);
pub const SLIDER_BORDER: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x33);
pub const SLIDER_FILL: Color32 = Color32::from_rgb(0x1a, 0x9f, 0xff);
pub const SLIDER_GRAB: Color32 = Color32::from_rgb(0xdc, 0xde, 0xdf);

// Scrollbar thumb: #434953, hover #58606d; 12px wide with 3px transparent borders (a 6px thumb).
pub const SCROLL_THUMB: Color32 = Color32::from_rgb(0x43, 0x49, 0x53);
pub const SCROLL_THUMB_HOVER: Color32 = Color32::from_rgb(0x58, 0x60, 0x6d);

// Modal dialogs (ModalPosition, DialogContent, DialogHeader, DialogLabel).
pub const MODAL_BG: Color32 = Color32::from_rgb(0x25, 0x28, 0x2e);
pub const MODAL_PAD: i8 = 24; // estimate
/// DialogHeader: 22px bold white. DialogControlsSectionHeader: 18px medium. DialogLabel: 13px, #acb2b8, caps.
pub const DIALOG_HEADER_SIZE: f32 = 22.0;
pub const SECTION_HEADER_SIZE: f32 = 18.0;
pub const DIALOG_LABEL: Color32 = Color32::from_rgb(0xac, 0xb2, 0xb8);
/// DialogToggle_Description / DialogLabelExplainer.
pub const DIALOG_DESC: Color32 = Color32::from_rgb(0x99, 0x9c, 0x9f);

/// Green button (Add to cart, dialogs): flat, the game page's PLAY green.
pub const PLAY_A: Color32 = Color32::from_rgb(0x4d, 0xc6, 0x34);
pub const PLAY_B: Color32 = Color32::from_rgb(0x4d, 0xc6, 0x34);

/// Friend names by state, as in the official friends list.
pub const FRIEND_INGAME: Color32 = Color32::from_rgb(0x90, 0xba, 0x3c); // estimate
pub const FRIEND_ONLINE: Color32 = Color32::from_rgb(0x6d, 0xcf, 0xf6); // friend .online
pub const FRIEND_AWAY: Color32 = Color32::from_rgb(0x4c, 0x91, 0xac); // friend away
pub const FRIEND_OFFLINE: Color32 = Color32::from_rgb(0x8b, 0x92, 0x9a);
/// Chat bubbles: ours and theirs.
pub const CHAT_MINE: Color32 = Color32::from_rgb(0x1f, 0x4b, 0x73); // estimate
pub const CHAT_THEIRS: Color32 = Color32::from_rgb(0x2f, 0x33, 0x3d); // estimate

// ---- sizes (points) -----------------------------------------------------

/// Control heights: no button is shorter than `CONTROL_SMALL`.
pub const CONTROL_SMALL: f32 = 28.0; // estimate
pub const CONTROL_NORMAL: f32 = 36.0; // estimate
pub const CONTROL_LARGE: f32 = 48.0; // estimate
pub const MENU_BAR_H: f32 = 34.0; // top 0-33 (measured: content starts at y 78 = 34 + 44)
pub const NAV_BAR_H: f32 = 44.0;
pub const BOTTOM_BAR_H: f32 = 49.0; // y 1030-1079 measured
pub const SIDEBAR_W: f32 = 273.0; // x 936-1208 measured
pub const FRIENDS_W: f32 = 320.0; // estimate
pub const LIST_ROW_H: f32 = 26.0; // measured row pitch
pub const CAPSULE_W: f32 = 150.0; // estimate
pub const CAPSULE_H: f32 = 225.0; // estimate
pub const GRID_GAP: f32 = 20.0; // estimate
/// Cover-size slider range (capsule width; the height is 3:2 of it).
pub const CAPSULE_W_MIN: f32 = 110.0; // estimate
pub const CAPSULE_W_MAX: f32 = 250.0; // estimate
/// "Recent games" shelf: the first capsule is larger, like the official library.
pub const RECENT_HERO_W: f32 = 300.0; // estimate
pub const RECENT_HERO_H: f32 = 140.0; // estimate
pub const RECENT_W: f32 = 220.0; // estimate
pub const RECENT_H: f32 = 103.0; // estimate
pub const SHELF_TITLE_H: f32 = 34.0; // estimate
pub const SHELF_BUCKET_H: f32 = 24.0; // estimate
/// Height of the "ALL GAMES" bar above the grid.
pub const ALL_BAR_H: f32 = 44.0; // estimate
pub const NAV_TAB_FONT: f32 = 17.5; // matched to the reference text widths (STORE 57 px, LIBRARY 73 px)
pub const WINDOW_BUTTON_W: f32 = 32.0;
/// Invisible zone along each window edge that starts a resize.
pub const RESIZE_EDGE: f32 = 6.0; // estimate
pub const STORE_CARD_W: f32 = 230.0; // estimate
pub const STORE_CARD_H: f32 = 107.0 + 30.0; // estimate
pub const PAGE_SLIDE: f32 = 8.0; // estimate
pub const PAGE_PAD: f32 = 24.0; // estimate
pub const HERO_ASPECT: f32 = 220.0 / 662.0; // measured hero height 220 at the reference width
/// Narrowest a menu gets, so short item lists do not look cramped.
pub const MENU_MIN_W: f32 = 200.0; // estimate
pub const SETTINGS_NAV_W: f32 = 260.0; // estimate
pub const SETTINGS_TITLE_H: f32 = 32.0; // estimate
pub const TOGGLE_W: f32 = 40.0; // estimate
pub const TOGGLE_H: f32 = 22.0; // estimate
pub const RADIUS: CornerRadius = CornerRadius::same(3); // measured: list/boxes 2-4 px
/// DialogButton radius.
pub const BUTTON_RADIUS: CornerRadius = CornerRadius::same(2);

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
/// Rows are full width: the panel has no side padding and the hover fill runs edge to edge. The hovered row
/// is inverted (light fill, dark text), as in the official client's menus.
pub fn menu_style(style: &mut Style) {
    style.spacing.button_padding = vec2(14.0, 4.0);
    style.spacing.item_spacing = vec2(0.0, 0.0);
    style.spacing.menu_margin = Margin::symmetric(0, 4);
    style.spacing.interact_size.y = 26.0;
    style.text_styles.insert(egui::TextStyle::Button, regular(14.0));
    style.text_styles.insert(egui::TextStyle::Body, regular(14.0));
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

/// The metrics of a settings page, dialog or Properties window: Steam's 32 px high controls (buttons, text
/// fields, drop-downs), 14 px text, and the `DialogButton` padding. Call it on the root `Ui` of such a window
/// (or inside a modal); everything drawn below inherits it.
pub fn dialog_style(ui: &mut egui::Ui) {
    let s = ui.style_mut();
    s.spacing.interact_size.y = BTN_H;
    s.spacing.button_padding = vec2(BTN_PAD_X, 0.0);
    s.spacing.item_spacing = vec2(8.0, 8.0);
    s.spacing.combo_width = 160.0;
    s.text_styles.insert(egui::TextStyle::Button, regular(BTN_FONT));
    s.text_styles.insert(egui::TextStyle::Body, regular(BTN_FONT));
}

/// Apply the base look to egui's own widgets (buttons, text edits, drop-downs, scrollbars, menus, tooltips) so
/// anything drawn with plain egui already looks like the official client.
pub fn apply(ctx: &egui::Context) {
    let mut v = Visuals::dark();
    v.panel_fill = BG_MAIN;
    // Popups and tooltips (dialogs and panels draw their own frames).
    v.window_fill = MENU_BG;
    v.window_stroke = Stroke::new(1.0, MENU_BORDER);
    v.popup_shadow = Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(90) };
    v.menu_corner_radius = CornerRadius::same(2);
    v.extreme_bg_color = BG_INPUT;
    // Text fields: Steam's dark inset (DialogInput); egui draws the focus outline with `selection.stroke`.
    v.text_edit_bg_color = Some(INPUT_BG);
    // No global text override: menus invert the text color on hover, which an override would forbid.
    v.override_text_color = None;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.linear_multiply(0.5);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.widgets.noninteractive.bg_stroke = Stroke::NONE;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    // DialogButton: flat #3d4450, #464d58 on hover, #393f49 pressed, no borders. `weak_bg_fill` is what an
    // egui `Button` paints; `bg_fill` is the scrollbar thumb (#434953, #58606d on hover).
    v.widgets.inactive.bg_fill = SCROLL_THUMB;
    v.widgets.inactive.weak_bg_fill = BTN_FLAT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, BTN_TEXT);
    v.widgets.hovered.bg_fill = SCROLL_THUMB_HOVER;
    v.widgets.hovered.weak_bg_fill = BTN_FLAT_HOVER;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.hovered.expansion = 0.0;
    v.widgets.active.bg_fill = SCROLL_THUMB_HOVER;
    v.widgets.active.weak_bg_fill = BTN_FLAT_ACTIVE;
    v.widgets.active.bg_stroke = Stroke::NONE;
    v.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    v.widgets.active.expansion = 0.0;
    v.widgets.open.weak_bg_fill = BTN_FLAT_HOVER;
    v.widgets.open.bg_stroke = Stroke::NONE;
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
        w.corner_radius = BUTTON_RADIUS;
    }
    // Sliders drawn by egui (the few left): round grabber, blue trail.
    v.handle_shape = egui::style::HandleShape::Circle;
    v.slider_trailing_fill = true;
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        // Steam's scrollbars: 12 px wide with a 6 px thumb; thin and invisible until the area is scrolled
        // or hovered in floating mode.
        s.spacing.scroll = scroll_style();
        s.spacing.menu_margin = Margin::symmetric(8, 5);
        s.spacing.slider_rail_height = 10.0;
        s.interaction.selectable_labels = false;
    });
}

// ---- pages (library, game page) ----
/// Game page action bar: button height, min width of the Play / Stop / Install button, and the square
/// icon buttons (gear, info, favourite) next to the stats.
pub const PAGE_ACTION_H: f32 = 33.0; // PLAY button height, measured y 308-340
pub const PAGE_PLAY_MIN_W: f32 = 93.0; // measured 1229-1322
pub const PAGE_ICON_BTN: f32 = 32.0; // measured 32x32, radius 4
/// Fill of the icon buttons (MenuButton: radius 4, fades in 0.12 s).
pub const PAGE_ICON_BTN_BG: Color32 = Color32::from_rgb(0x3d, 0x3e, 0x44); // measured
pub const PAGE_ICON_BTN_BG_HOVER: Color32 = Color32::from_rgb(0x4a, 0x4d, 0x54);
/// PLAY button (measured): the main part, and the drop-down arrow segment next to it (a different, teal green).
pub const PAGE_PLAY_A: Color32 = Color32::from_rgb(0x4d, 0xc6, 0x34);
pub const PAGE_PLAY_B: Color32 = Color32::from_rgb(0x18, 0xb1, 0x4c);
/// Stop / Install blue (flat).
pub const PAGE_STOP_A: Color32 = Color32::from_rgb(0x19, 0x9a, 0xff);
pub const PAGE_STOP_B: Color32 = Color32::from_rgb(0x19, 0x9a, 0xff);
/// Stat columns (PlayBarLabel / PlayBarDetailLabel).
pub const PAGE_STAT_LABEL: f32 = 14.0; // PlayBarLabel 14 px caps, letter-spacing 1, white 52 %
pub const PAGE_STAT_VALUE: f32 = 13.0; // PlayBarDetailLabel 13 px, white 32 %
pub const PAGE_STAT_LABEL_COLOR: Color32 = Color32::from_rgb(0xa1, 0xa0, 0xa3); // measured
pub const PAGE_STAT_VALUE_COLOR: Color32 = Color32::from_rgb(0x75, 0x77, 0x7b); // measured
/// Game page tab strip (Store Page, DLC, Community Hub ...).
pub const PAGE_TAB_H: f32 = 38.0; // tab box y 357-395, measured
pub const PAGE_TAB_FONT: f32 = 14.0; // estimate
/// Left sidebar header rows.
pub const SIDE_HEADER_H: f32 = 32.0; // estimate
pub const SIDE_ICON_BTN: f32 = 24.0; // estimate
/// Sidebar game-list text.
pub const SIDE_ROW_FONT: f32 = 14.0; // estimate

// ---- new Steam surfaces (measured from the reference screenshot) ----
/// Sidebar boxes: Home row, "Games and Software" drop-down, grid button.
pub const SIDE_BOX: Color32 = Color32::from_rgb(0x26, 0x27, 0x2e);
/// Sort button next to the drop-down.
pub const SIDE_SORT_BTN: Color32 = Color32::from_rgb(0x34, 0x37, 0x3f);
/// Grid button glyph.
pub const SIDE_GRID_ICON: Color32 = Color32::from_rgb(0x4b, 0x4d, 0x51);
/// Dim icons (search magnifier, sort icons) and the text of the drop-down.
pub const SIDE_ICON: Color32 = Color32::from_rgb(0x87, 0x8b, 0x91);
pub const SIDE_TEXT: Color32 = Color32::from_rgb(0xb3, 0xb5, 0xbb);
/// Game rows: installed, and the text of a selected row.
pub const ROW_TEXT: Color32 = Color32::from_rgb(0xf6, 0xfb, 0xff);
/// Collection header text and count.
pub const HEADER_TEXT: Color32 = Color32::from_rgb(0xdb, 0xe6, 0xf4);
pub const HEADER_COUNT: Color32 = Color32::from_rgb(0x53, 0x5b, 0x66);
/// Top bar: the announcements button, the bell button, the account pill (all measured).
pub const TOP_BELL_BTN: Color32 = Color32::from_rgb(0x26, 0x2c, 0x35);
pub const TOP_ACCOUNT: Color32 = Color32::from_rgb(0x29, 0x2e, 0x37);
pub const TOP_ICON: Color32 = Color32::from_rgb(0x89, 0x91, 0x99);
pub const TOP_WALLET: Color32 = Color32::from_rgb(0x83, 0x8b, 0x93);
pub const TOP_NAME: Color32 = Color32::from_rgb(0xaf, 0xb4, 0xbd);
/// Game page: tab box fill/border, panel (recommend box) fill.
pub const TAB_BOX_BG: Color32 = Color32::from_rgb(0x2e, 0x33, 0x3b);
pub const TAB_BOX_BORDER: Color32 = Color32::from_rgb(0x32, 0x38, 0x3f);
pub const TAB_TEXT: Color32 = Color32::from_rgb(0x7d, 0x80, 0x87);
pub const PANEL_BG: Color32 = Color32::from_rgb(0x43, 0x49, 0x52);
/// Bottom bar text.
pub const BOTTOM_TEXT: Color32 = Color32::from_rgb(0x8b, 0x92, 0x9a);
/// Search row fill (measured).
pub const SIDE_SEARCH_BG: Color32 = Color32::from_rgb(0x25, 0x26, 0x2c);
/// Dark strip behind the Home row (measured).
pub const SIDE_HOME_ZONE: Color32 = Color32::from_rgb(0x19, 0x1c, 0x24);
/// Darker panels on the game page (post-game summary area, measured).
pub const BG_PANEL_DARK: Color32 = Color32::from_rgb(0x20, 0x23, 0x29);
/// Game page background: flat down to the middle, then darker towards the bottom (measured at x 1215, y 390-1000).
pub const PAGE_BG_TOP: Color32 = Color32::from_rgb(0x28, 0x2c, 0x31);
pub const PAGE_BG_BOTTOM: Color32 = Color32::from_rgb(0x1b, 0x1d, 0x1f);
/// 2 px dark line between the sidebar and the page (measured #14181b at x 1207-1208).
pub const DIVIDER: Color32 = Color32::from_rgb(0x14, 0x18, 0x1b);
/// Grid button next to Home (measured).
pub const SIDE_GRID_BG: Color32 = Color32::from_rgb(0x28, 0x2a, 0x2f);
/// Collection header row (measured at (1100,597)).
pub const HEADER_BG: Color32 = Color32::from_rgb(0x27, 0x2d, 0x36);
/// Menu bar text (measured peak).
pub const TOP_MENU_TEXT: Color32 = Color32::from_rgb(0x89, 0x8f, 0x96);
/// Top of the sidebar below the Home strip is lighter and fades into the list colour over about 280 px (measured
/// #2d3038 at y 124, #292e35 at y 200, #242830 from y 400).
pub const SIDE_BG_TOP: Color32 = Color32::from_rgb(0x2c, 0x30, 0x37);

// ---- game page, below the tab box (sampled from Steam, TF2 page in steam-vs-kettle.webp) ----
pub const GP_FEATURED_HEADER: Color32 = Color32::from_rgb(0x1b, 0x61, 0x92); // sampled from Steam: "FEATURED" strip
pub const GP_FEATURED_BORDER: Color32 = Color32::from_rgb(0x4a, 0x7a, 0x9e); // sampled from Steam: 1 px outline (#5a83a3 left, #3a5567 bottom)
pub const GP_FEATURED_BODY: Color32 = Color32::from_rgb(0x2a, 0x2f, 0x36); // sampled from Steam (#282c31 left, #2c3239 right)
pub const GP_FEATURED_TEXT: Color32 = Color32::from_rgb(0xe8, 0xff, 0xff); // sampled from Steam: "FEATURED"
pub const GP_FEATURED_TYPE: Color32 = Color32::from_rgb(0x96, 0x9c, 0xa3); // sampled from Steam: "SMALL UPDATE / PATCH NOTES"
pub const GP_FEATURED_TITLE: Color32 = Color32::from_rgb(0xd0, 0xd3, 0xd9); // sampled from Steam: event title
pub const GP_SECTION_TITLE: Color32 = Color32::from_rgb(0xa0, 0xa4, 0xa7); // sampled from Steam: ACTIVITY / FRIENDS WHO PLAY
pub const GP_BOX_BG: Color32 = Color32::from_rgb(0x27, 0x2c, 0x32); // sampled from Steam: friends box (#262c31 top, #2a2f37 bottom)
pub const GP_BOX_BORDER: Color32 = Color32::from_rgb(0x3a, 0x40, 0x47); // sampled from Steam: friends box edge
pub const GP_BOX_TEXT: Color32 = Color32::from_rgb(0xcb, 0xcf, 0xd2); // sampled from Steam: "1 friend is playing right now"
pub const GP_FRIEND_NAME: Color32 = Color32::from_rgb(0xec, 0xf8, 0xe6); // sampled from Steam: friend in game
pub const GP_FRIEND_RICH: Color32 = Color32::from_rgb(0xa0, 0xb2, 0x9a); // sampled from Steam: rich presence line
pub const GP_LINK: Color32 = Color32::from_rgb(0x8f, 0x94, 0x98); // sampled from Steam: "View all friends who play", "View Latest News"
pub const GP_DATE_HEAD: Color32 = Color32::from_rgb(0x8c, 0x8e, 0x90); // sampled from Steam: "TODAY"
pub const GP_DATE_LINE: Color32 = Color32::from_rgb(0x32, 0x33, 0x37); // sampled from Steam: rule above the date
pub const GP_FEATURED_H: f32 = 100.0; // measured: card y 425-525
pub const GP_FEATURED_HEADER_H: f32 = 27.0; // measured: strip y 425-452
pub const GP_RIGHT_W: f32 = 187.0; // measured: right column x 715-902
pub const GP_COL_GAP: f32 = 32.0; // measured: left column ends x 683

/// Steam's scrollbar: no track, and the thin thumb shows only while the pointer is over the scrolled area
/// (or the bar is dragged); it widens to 12 px with the pointer on the bar.
pub fn scroll_style() -> egui::style::ScrollStyle {
    egui::style::ScrollStyle {
        bar_width: 12.0,
        floating_width: 3.0,
        floating_allocated_width: 6.0,
        foreground_color: false,
        dormant_background_opacity: 0.0,
        dormant_handle_opacity: 0.0,
        active_background_opacity: 0.0,
        active_handle_opacity: 1.0,
        interact_background_opacity: 0.0,
        interact_handle_opacity: 1.0,
        ..egui::style::ScrollStyle::floating()
    }
}
// ---- recommendation box and post-game summary (sampled from Steam, TF2 page in steam-vs-kettle.webp) ----
pub const REC_BG: Color32 = Color32::from_rgb(0x43, 0x49, 0x52); // box fill, x 1229-1837, y 415-505
pub const REC_BTN: Color32 = Color32::from_rgb(0x4a, 0x61, 0x72); // Yes / No / Maybe Later fill
pub const REC_BTN_HOVER: Color32 = Color32::from_rgb(0x58, 0x72, 0x86);
pub const REC_BTN_TEXT: Color32 = Color32::from_rgb(0x85, 0xbc, 0xe2); // button text and thumbs
pub const REC_TITLE: Color32 = Color32::from_rgb(0x7f, 0xaf, 0xd2); // "You've played for N hours"
pub const REC_BODY: Color32 = Color32::from_rgb(0xf8, 0xfd, 0xff); // "Would you recommend this game ..."
pub const REC_H: f32 = 90.0;
pub const POST_PANEL_BG: Color32 = Color32::from_rgb(0x16, 0x19, 0x1e); // post-game strip (flat; Steam's is #15191c-#17181f)
pub const POST_PANEL_H: f32 = 185.0; // y 553-738
pub const POST_THUMB_W: f32 = 224.0;
pub const POST_THUMB_H: f32 = 126.0;
pub const POST_THUMB_GAP: f32 = 22.0;
pub const POST_ICON: Color32 = Color32::from_rgb(0xd4, 0xd8, 0xdd); // < > X
pub const POST_ICON_OFF: Color32 = Color32::from_rgb(0x5a, 0x60, 0x68);
pub const POST_DAY: Color32 = Color32::from_rgb(0xcb, 0xcf, 0xd4); // "Today"
pub const SAY_BG: Color32 = Color32::from_rgb(0x1d, 0x21, 0x27); // "Say something ..." field
pub const SAY_HINT: Color32 = Color32::from_rgb(0x66, 0x6c, 0x71);
pub const SAY_H: f32 = 36.0; // y 930-966
