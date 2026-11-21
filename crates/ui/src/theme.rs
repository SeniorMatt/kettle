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
pub const BG_TOP: Color32 = Color32::from_rgb(0x17, 0x1a, 0x21); // estimate
/// Library content area.
pub const BG_MAIN: Color32 = Color32::from_rgb(0x1b, 0x1e, 0x25); // estimate
/// Left game list.
pub const BG_SIDEBAR: Color32 = Color32::from_rgb(0x23, 0x26, 0x2e); // palette #23262e
/// Bottom bar (downloads, friends).
pub const BG_BOTTOM: Color32 = Color32::from_rgb(0x1e, 0x21, 0x28); // estimate
/// Text inputs (search).
pub const BG_INPUT: Color32 = Color32::from_rgb(0x32, 0x35, 0x3c); // estimate
/// Hovered and selected rows.
pub const BG_ROW_HOVER: Color32 = Color32::from_rgb(0x32, 0x36, 0x40); // estimate
pub const BG_ROW_SELECTED: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50); // palette #3d4450
/// Placeholder behind a cover that hasn't loaded.
pub const BG_CAPSULE: Color32 = Color32::from_rgb(0x2a, 0x2e, 0x37); // estimate
/// Bar under the hero image on a game page.
pub const BG_ACTION_BAR: Color32 = Color32::from_rgb(0x20, 0x24, 0x2c); // estimate

pub const TEXT: Color32 = Color32::from_rgb(0xdc, 0xde, 0xdf); // palette #dcdedf
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8b, 0x92, 0x9a); // palette #8b929a
/// Not-installed games in the list.
pub const TEXT_UNINSTALLED: Color32 = Color32::from_rgb(0x6e, 0x75, 0x7e); // estimate
pub const TEXT_HEADING: Color32 = Color32::from_rgb(0xb8, 0xbc, 0xbf); // palette #b8bcbf
/// Hairline around the window (we draw our own frame).
pub const WINDOW_BORDER: Color32 = Color32::from_rgb(0x2c, 0x31, 0x3b); // estimate
/// Close button hover.
pub const CLOSE_HOVER: Color32 = Color32::from_rgb(0xe8, 0x11, 0x23); // estimate
/// Store discount box (dark green with light green text, like the store).
pub const DISCOUNT_BG: Color32 = Color32::from_rgb(0x4c, 0x6b, 0x22); // estimate
pub const DISCOUNT_TEXT: Color32 = Color32::from_rgb(0xbe, 0xee, 0x11); // estimate
/// Price tag background next to a discount box.
pub const PRICE_BG: Color32 = Color32::from_rgb(0x30, 0x3a, 0x44); // estimate
/// Library home: the faint blue glow at the top of the page.
pub const HOME_GLOW: Color32 = Color32::from_rgb(0x2a, 0x3a, 0x50); // estimate
/// Selected game in the list: a blue gradient fading to the right.
pub const ROW_SELECTED_A: Color32 = Color32::from_rgb(0x2f, 0x5f, 0x8e); // estimate
pub const ROW_SELECTED_B: Color32 = Color32::from_rgb(0x23, 0x2e, 0x40); // estimate
/// "Running" label in the game list.
pub const RUNNING: Color32 = Color32::from_rgb(0x90, 0xba, 0x3c); // estimate
pub const ACCENT: Color32 = Color32::from_rgb(0x1a, 0x9f, 0xff); // palette #1a9fff
/// Destructive buttons (uninstall) and error text.
pub const DANGER: Color32 = Color32::from_rgb(0xa8, 0x32, 0x32); // estimate
pub const ERROR_TEXT: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x5c); // estimate
/// Download progress for verifying (hashing) instead of fetching.
pub const VERIFY: Color32 = Color32::from_rgb(0x75, 0xb0, 0x22); // estimate
/// A card on a page (download rows, settings groups).
pub const BG_CARD: Color32 = Color32::from_rgb(0x23, 0x27, 0x30); // estimate

// Menus, drop-down lists and tooltips: a flat slate panel; the hovered row is inverted, like the official client.
pub const MENU_BG: Color32 = Color32::from_rgb(0x3d, 0x44, 0x50); // palette #3d4450
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
/// DialogButton.Primary and the install button: gradient #47bfff -> #1a44c2 (stops 0 % and 60 %, sliding).
pub const BTN_PRIMARY_A: Color32 = Color32::from_rgb(0x47, 0xbf, 0xff);
pub const BTN_PRIMARY_B: Color32 = Color32::from_rgb(0x1a, 0x44, 0xc2);
/// `button.deleteButton`: #b14040 -> #742a29.
pub const BTN_DANGER_A: Color32 = Color32::from_rgb(0xb1, 0x40, 0x40);
pub const BTN_DANGER_B: Color32 = Color32::from_rgb(0x74, 0x2a, 0x29);

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

// DialogSlider: track #202226 in a 4px #2a2d33 border (10px high), value #00ccff -> #2d73ff, grabber 12px.
pub const SLIDER_TRACK: Color32 = Color32::from_rgb(0x20, 0x22, 0x26);
pub const SLIDER_BORDER: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x33);
pub const SLIDER_FILL_A: Color32 = Color32::from_rgb(0x00, 0xcc, 0xff);
pub const SLIDER_FILL_B: Color32 = Color32::from_rgb(0x2d, 0x73, 0xff);
pub const SLIDER_GRAB_A: Color32 = Color32::from_rgb(0x89, 0xe7, 0xff);
pub const SLIDER_GRAB_B: Color32 = Color32::from_rgb(0x80, 0xaa, 0xff);

// Scrollbar thumb: #434953, hover #58606d; 12px wide with 3px transparent borders (a 6px thumb).
pub const SCROLL_THUMB: Color32 = Color32::from_rgb(0x43, 0x49, 0x53);
pub const SCROLL_THUMB_HOVER: Color32 = Color32::from_rgb(0x58, 0x60, 0x6d);

// Modal dialogs (ModalPosition, DialogContent, DialogHeader, DialogLabel).
pub const MODAL_BG: Color32 = Color32::from_rgb(0x25, 0x28, 0x2e);
/// ModalPosition_TopBar: 1px gradient across the top.
pub const MODAL_BAR_A: Color32 = Color32::from_rgb(0x00, 0xcc, 0xff);
pub const MODAL_BAR_B: Color32 = Color32::from_rgb(0x33, 0x66, 0xff);
pub const MODAL_PAD: i8 = 24; // estimate
/// DialogHeader: 22px bold white. DialogControlsSectionHeader: 18px medium. DialogLabel: 13px, #acb2b8, caps.
pub const DIALOG_HEADER_SIZE: f32 = 22.0;
pub const SECTION_HEADER_SIZE: f32 = 18.0;
pub const DIALOG_LABEL: Color32 = Color32::from_rgb(0xac, 0xb2, 0xb8);
/// DialogToggle_Description / DialogLabelExplainer.
pub const DIALOG_DESC: Color32 = Color32::from_rgb(0x99, 0x9c, 0x9f);

/// Play button gradient: `button.GreenPlay` rgb(138,195,41) -> rgb(74,122,22).
pub const PLAY_A: Color32 = Color32::from_rgb(138, 195, 41);
pub const PLAY_B: Color32 = Color32::from_rgb(74, 122, 22);
/// Install button gradient (same as the primary button).
pub const INSTALL_A: Color32 = Color32::from_rgb(0x47, 0xbf, 0xff);
pub const INSTALL_B: Color32 = Color32::from_rgb(0x1a, 0x44, 0xc2);

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
pub const MENU_BAR_H: f32 = 28.0; // estimate
pub const NAV_BAR_H: f32 = 52.0; // estimate
pub const BOTTOM_BAR_H: f32 = 40.0; // estimate
pub const SIDEBAR_W: f32 = 300.0; // estimate
pub const FRIENDS_W: f32 = 320.0; // estimate
pub const LIST_ROW_H: f32 = 28.0; // estimate
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
pub const NAV_TAB_FONT: f32 = 22.0; // estimate
pub const WINDOW_BUTTON_W: f32 = 44.0; // estimate
/// Invisible zone along each window edge that starts a resize.
pub const RESIZE_EDGE: f32 = 6.0; // estimate
pub const STORE_CARD_W: f32 = 230.0; // estimate
pub const STORE_CARD_H: f32 = 107.0 + 30.0; // estimate
pub const PAGE_SLIDE: f32 = 8.0; // estimate
pub const PAGE_PAD: f32 = 24.0; // estimate
pub const HERO_ASPECT: f32 = 1240.0 / 3840.0; // estimate
/// Narrowest a menu gets, so short item lists do not look cramped.
pub const MENU_MIN_W: f32 = 200.0; // estimate
pub const SETTINGS_NAV_W: f32 = 260.0; // estimate
pub const SETTINGS_TITLE_H: f32 = 32.0; // estimate
pub const TOGGLE_W: f32 = 40.0; // estimate
pub const TOGGLE_H: f32 = 22.0; // estimate
pub const RADIUS: CornerRadius = CornerRadius::same(2); // estimate
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
        s.spacing.scroll =
            egui::style::ScrollStyle { bar_width: 12.0, floating_width: 3.0, ..egui::style::ScrollStyle::thin() };
        s.spacing.menu_margin = Margin::symmetric(8, 5);
        s.spacing.slider_rail_height = 10.0;
        s.interaction.selectable_labels = false;
    });
}

// ---- pages (library, game page) ----
/// Game page action bar: button height, min width of the Play / Stop / Install button, and the square
/// icon buttons (gear, info, favourite) next to the stats.
pub const PAGE_ACTION_H: f32 = 46.0; // ActionSection height 46
pub const PAGE_PLAY_MIN_W: f32 = 164.0; // ClaimButtonContainer min-width 164
pub const PAGE_ICON_BTN: f32 = 32.0; // MenuButton 32x32, radius 4
/// Fill of the icon buttons (MenuButton: radius 4, fades in 0.12 s).
pub const PAGE_ICON_BTN_BG: Color32 = Color32::from_rgba_premultiplied(24, 25, 28, 36); // rgba(172,178,201,.141)
pub const PAGE_ICON_BTN_BG_HOVER: Color32 = Color32::from_rgba_premultiplied(53, 55, 59, 66); // rgba(205,213,226,.26)
/// Play button gradient (stops at 0 % and 60 %, slid like `DialogButton`) and the Stop / Install blue.
pub const PAGE_PLAY_A: Color32 = Color32::from_rgb(0x70, 0xd6, 0x1d); // PlayBar green #70d61d -> #01a75b (stop 60 %)
pub const PAGE_PLAY_B: Color32 = Color32::from_rgb(0x01, 0xa7, 0x5b);
pub const PAGE_STOP_A: Color32 = Color32::from_rgb(0x47, 0xbf, 0xff); // estimate
pub const PAGE_STOP_B: Color32 = Color32::from_rgb(0x1a, 0x44, 0xc2); // estimate
/// Stat columns (PlayBarLabel / PlayBarDetailLabel).
pub const PAGE_STAT_LABEL: f32 = 14.0; // PlayBarLabel 14 px caps, letter-spacing 1, white 52 %
pub const PAGE_STAT_VALUE: f32 = 13.0; // PlayBarDetailLabel 13 px, white 32 %
pub const PAGE_STAT_LABEL_COLOR: Color32 = Color32::from_rgba_premultiplied(133, 133, 133, 133);
/// Game page tab strip (Store Page, DLC, Community Hub ...).
pub const PAGE_TAB_H: f32 = 40.0; // estimate
pub const PAGE_TAB_FONT: f32 = 14.0; // estimate
/// Left sidebar header rows.
pub const SIDE_HEADER_H: f32 = 32.0; // estimate
pub const SIDE_ICON_BTN: f32 = 24.0; // estimate
/// Sidebar game-list text.
pub const SIDE_ROW_FONT: f32 = 14.0; // estimate
