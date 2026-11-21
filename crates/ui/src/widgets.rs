//! Small reusable widgets drawn with the painter.

use crate::{anim, theme};
use egui::{Color32, Pos2, Rect, Response, Sense, Ui, Vec2, epaint::Mesh};

/// Fill `rect` with a left-to-right gradient.
pub fn gradient(ui: &Ui, rect: Rect, left: Color32, right: Color32) {
    let mut mesh = Mesh::default();
    let i = mesh.vertices.len() as u32;
    for (pos, c) in
        [(rect.left_top(), left), (rect.right_top(), right), (rect.right_bottom(), right), (rect.left_bottom(), left)]
    {
        mesh.colored_vertex(pos, c);
    }
    mesh.add_triangle(i, i + 1, i + 2);
    mesh.add_triangle(i, i + 2, i + 3);
    ui.painter().add(mesh);
}

/// A flat coloured button with an icon and an upper-case label (store "Add to cart"): 2 px radius, no gradient.
pub fn action_button(ui: &mut Ui, icon: &str, label: &str, colors: (Color32, Color32)) -> Response {
    let font = theme::regular(16.0);
    let text = format!("{icon}  {}", label.to_uppercase());
    let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
    let size = Vec2::new((galley.size().x + 32.0).max(theme::PAGE_PLAY_MIN_W), theme::PAGE_ACTION_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), 0.12);
    let press = resp.is_pointer_button_down_on();
    let bg = shade(colors.0, hover, press);
    ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, bg);
    let at = rect.center() - galley.size() / 2.0;
    ui.painter().galley(at, galley, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A flat colour brightened on hover and darkened while pressed.
pub fn shade(base: Color32, hover: f32, press: bool) -> Color32 {
    let c = base.lerp_to_gamma(Color32::WHITE, 0.12 * hover);
    if press { c.gamma_multiply(0.85) } else { c }
}

/// A square icon button (gear, info, ...): `MenuButton`, radius 4, translucent fill that brightens on hover.
pub fn icon_button(ui: &mut Ui, icon: &str, size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let t = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), 0.12);
    let bg = theme::PAGE_ICON_BTN_BG.lerp_to_gamma(theme::PAGE_ICON_BTN_BG_HOVER, t);
    ui.painter().rect_filled(rect, 4.0, bg);
    let color = theme::SIDE_ICON.lerp_to_gamma(Color32::WHITE, t);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, theme::regular(size * 0.55), color);
    resp.on_hover_text(tooltip)
}

/// A soft light band sweeping over a loading placeholder. Repaints (about 20
/// times a second) only while it is drawn, i.e. while it is visible and loading.
pub fn shimmer(ui: &Ui, rect: Rect) {
    const PERIOD: f64 = 1.4;
    let t = (ui.input(|i| i.time) % PERIOD / PERIOD) as f32;
    let band = rect.width() * 0.6;
    let x = rect.left() - band + (rect.width() + band) * t;
    let (x0, x1) = (x.max(rect.left()), (x + band).min(rect.right()));
    if x1 > x0 {
        let centre = x + band / 2.0;
        let glow =
            |v: f32| Color32::from_white_alpha((12.0 * (1.0 - (v - centre).abs() / (band / 2.0)).max(0.0)) as u8);
        let mid = centre.clamp(x0, x1);
        for (a, b) in [(x0, mid), (mid, x1)] {
            if b > a {
                let r = Rect::from_min_max(Pos2::new(a, rect.top()), Pos2::new(b, rect.bottom()));
                gradient(ui, r, glow(a), glow(b));
            }
        }
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
}

/// Placeholder for an image that is still loading.
pub fn placeholder(ui: &Ui, rect: Rect) {
    ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_CAPSULE);
    shimmer(ui, rect);
}

/// Paint `tex` into `rect`, cropping to keep its aspect ratio ("cover").
pub fn image_cover(ui: &Ui, rect: Rect, tex: &egui::TextureHandle, tint: Color32) {
    let [w, h] = tex.size().map(|v| v as f32);
    let (img_aspect, rect_aspect) = (w / h, rect.width() / rect.height());
    let uv = if img_aspect > rect_aspect {
        let u = rect_aspect / img_aspect;
        Rect::from_min_max(Pos2::new((1.0 - u) / 2.0, 0.0), Pos2::new((1.0 + u) / 2.0, 1.0))
    } else {
        let v = img_aspect / rect_aspect;
        Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, v))
    };
    ui.painter().image(tex.id(), rect, uv, tint);
}

/// Paint `tex` inside `bounds`, scaled to fit, anchored bottom-left.
pub fn image_fit_bottom_left(ui: &Ui, bounds: Rect, tex: &egui::TextureHandle, alpha: f32) {
    let [w, h] = tex.size().map(|v| v as f32);
    let s = (bounds.width() / w).min(bounds.height() / h);
    let size = Vec2::new(w * s, h * s);
    let rect = Rect::from_min_size(Pos2::new(bounds.left(), bounds.bottom() - size.y), size);
    ui.painter().image(
        tex.id(),
        rect,
        Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
        Color32::WHITE.gamma_multiply(alpha),
    );
}

/// Look of a [`button`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    /// Flat blue, for the main action of a dialog (`DialogButton.Primary`).
    Primary,
    /// Grey, for everything else (`DialogButton`).
    Secondary,
    /// Flat red, for destructive actions.
    Danger,
    /// Flat green.
    #[allow(dead_code)] // Kept with the other Steam button styles; nothing uses it yet.
    Green,
}

/// A Steam `DialogButton`: 32 px high, 2 px radius, 14 px text, grey (flat blue / green / red for the
/// coloured ones) that fades to its hover colour in 0.2 s. `enabled == false` draws it dim and inert.
pub fn button(ui: &mut Ui, label: &str, style: ButtonStyle, enabled: bool) -> Response {
    button_min(ui, label, style, enabled, 0.0)
}

/// [`button`] with a minimum width (dialog footers use [`theme::BTN_MIN_W_DIALOG`]).
pub fn button_min(ui: &mut Ui, label: &str, style: ButtonStyle, enabled: bool, min_w: f32) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(theme::BTN_FONT), theme::BTN_TEXT);
    let size = Vec2::new((galley.size().x + 2.0 * theme::BTN_PAD_X).max(min_w), theme::BTN_H);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered() && enabled, 0.2);
    let press = anim::animate_bool(ui.ctx(), resp.id.with("press"), resp.is_pointer_button_down_on(), 0.04);
    let radius = theme::BUTTON_RADIUS;
    let mut fg = Color32::WHITE;
    if !enabled {
        ui.painter().rect_filled(rect, radius, theme::BTN_DISABLED_BG);
        fg = theme::BTN_DISABLED_FG;
    } else if let Some(color) = match style {
        ButtonStyle::Primary => Some(theme::BTN_PRIMARY_A),
        ButtonStyle::Green => Some(theme::PLAY_A),
        ButtonStyle::Danger => Some(theme::BTN_DANGER_A),
        ButtonStyle::Secondary => None,
    } {
        ui.painter().rect_filled(rect, radius, shade(color, hover, press > 0.5));
    } else {
        let base = theme::BTN_FLAT.lerp_to_gamma(theme::BTN_FLAT_HOVER, hover);
        let bg = base.lerp_to_gamma(theme::BTN_FLAT_ACTIVE, press);
        ui.painter().rect_filled(rect, radius, bg);
        fg = theme::BTN_TEXT.lerp_to_gamma(Color32::WHITE, hover);
    }
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, fg);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

/// The row of buttons at the bottom of a dialog: right-aligned, in Steam's order. Add the primary button
/// first (it lands on the right), then the others; each goes to the left of the previous one.
pub fn button_row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        add(ui);
    });
}

/// A horizontal progress bar: dark track, accent fill. `fraction` is 0..1 and is drawn as given, so the
/// caller smooths it (see [`anim::glide`]).
pub fn progress_bar(ui: &mut Ui, width: f32, height: f32, fraction: f32, fill: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.painter().rect_filled(rect, theme::RADIUS, theme::BG_TOP);
    let w = rect.width() * fraction.clamp(0.0, 1.0);
    if w > 0.5 {
        let bar = Rect::from_min_size(rect.min, Vec2::new(w, rect.height()));
        ui.painter().rect_filled(bar, theme::RADIUS, fill);
    }
    resp
}

/// Small caps section heading ("UP NEXT", "RECENTLY FINISHED").
pub fn section_title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::bold(12.0)).color(theme::TEXT_HEADING));
}

/// A text link: dim, accent on hover.
pub fn link(ui: &mut Ui, text: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), theme::regular(13.0), theme::TEXT_DIM);
    let (rect, resp) = ui.allocate_exact_size(galley.size(), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    let color = theme::TEXT_DIM.lerp_to_gamma(theme::ACCENT, hover);
    ui.painter().galley_with_override_text_color(rect.min, galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

// ---- menus and drop-downs ---------------------------------------------------

/// A menu button for the menu bar and for menus inside forms: the popup gets [`theme::menu_style`].
pub fn menu_button(ui: &mut Ui, text: impl Into<egui::WidgetText>, content: impl FnOnce(&mut Ui)) -> Response {
    let config = egui::containers::menu::MenuConfig::new().style(theme::menu_style);
    egui::containers::menu::MenuButton::new(text.into())
        .config(config)
        .ui(ui, |ui| {
            ui.set_min_width(theme::MENU_MIN_W);
            content(ui);
        })
        .0
}

/// A popup menu opened by clicking `anchor` (for example the gear button), in the shared menu style.
pub fn popup_menu(anchor: &Response, content: impl FnOnce(&mut Ui)) {
    egui::Popup::menu(anchor).style(theme::menu_style).show(|ui| {
        ui.set_min_width(theme::MENU_MIN_W);
        content(ui);
    });
}

/// A right-click menu on `target`, in the shared menu style. Call it every frame; it draws only while open.
pub fn context_menu(target: &Response, content: impl FnOnce(&mut Ui)) {
    egui::Popup::context_menu(target).style(theme::menu_style).show(|ui| {
        ui.set_min_width(theme::MENU_MIN_W);
        content(ui);
    });
}

/// A thin line between groups of menu items.
pub fn menu_separator(ui: &mut Ui) {
    // The menu is as wide as its widest item so far: asking for all available width would stretch it
    // across the screen.
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.min_rect().width(), 9.0), Sense::hover());
    let y = rect.center().y.round();
    ui.painter().hline(rect.x_range(), y, egui::Stroke::new(1.0, theme::MENU_SEP));
}

/// Plain information in a menu (version line), padded like the items.
pub fn menu_label(ui: &mut Ui, text: &str, dim: bool) {
    let color = if dim { theme::MENU_SELECTED.lerp_to_gamma(Color32::WHITE, 0.45) } else { theme::TEXT };
    let padding = ui.spacing().button_padding;
    ui.add_space(padding.y);
    ui.horizontal(|ui| {
        ui.add_space(padding.x);
        ui.label(egui::RichText::new(text).color(color));
        ui.add_space(padding.x);
    });
    ui.add_space(padding.y);
}

/// A Steam `DialogDropDown`: dark field with 3 px radius, 14 px text, a small arrow on the right that
/// turns lighter on hover; the list opens as a menu in the shared menu style.
pub fn combo<R>(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    width: f32,
    selected: impl Into<egui::WidgetText>,
    list: impl FnOnce(&mut Ui) -> R,
) {
    // Never wider than the space it is given (settings rows leave the control about a third of the row).
    let width = width.min((ui.available_width() - 32.0).max(60.0));
    ui.scope(|ui| {
        let s = ui.style_mut();
        s.spacing.button_padding = Vec2::new(14.0, 0.0);
        s.spacing.interact_size.y = s.spacing.interact_size.y.max(theme::BTN_H);
        for (w, fill) in [
            (&mut s.visuals.widgets.inactive, theme::INPUT_BG),
            (&mut s.visuals.widgets.hovered, theme::DROPDOWN_HOVER),
            (&mut s.visuals.widgets.active, theme::DROPDOWN_HOVER),
            (&mut s.visuals.widgets.open, theme::DROPDOWN_HOVER),
        ] {
            w.weak_bg_fill = fill;
            w.corner_radius = theme::INPUT_RADIUS;
        }
        egui::ComboBox::from_id_salt(id)
            .width(width)
            .selected_text(selected)
            .popup_style(egui::style::StyleModifier::from(theme::menu_style))
            .icon(|ui, rect, visuals, _open| {
                let lit = visuals.fg_stroke.color == Color32::WHITE;
                let color = if lit { Color32::WHITE } else { theme::DROPDOWN_ARROW };
                crate::steam_icons::chevron_down(ui.painter(), rect.center(), 10.0, color);
            })
            .show_ui(ui, list);
    });
}

// ---- settings controls ------------------------------------------------------

/// A Steam-style switch: a pill with a knob that slides over (about 120 ms). Returns the click response;
/// the value is flipped in place.
pub fn toggle(ui: &mut Ui, on: &mut bool) -> Response {
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(theme::TOGGLE_W, theme::TOGGLE_H), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = anim::animate_bool(ui.ctx(), resp.id.with("on"), *on, 0.12);
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    let track = theme::TOGGLE_OFF.lerp_to_gamma(theme::TOGGLE_ON, t).lerp_to_gamma(Color32::WHITE, 0.08 * hover);
    ui.painter().rect_filled(rect, theme::TOGGLE_H / 2.0, track);
    let r = theme::TOGGLE_H / 2.0 - 3.0;
    let x = rect.left() + theme::TOGGLE_H / 2.0 + (theme::TOGGLE_W - theme::TOGGLE_H) * t;
    ui.painter().circle_filled(Pos2::new(x, rect.center().y), r, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A Steam `DialogCheckbox` with its label: an 18 px dark box with a white tick. The whole row is clickable.
pub fn checkbox(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::regular(theme::BTN_FONT), theme::BTN_TEXT);
    let box_size = theme::CHECK_SIZE;
    let size = Vec2::new(box_size + 8.0 + galley.size().x, box_size.max(galley.size().y).max(24.0));
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), 0.16);
    let t = anim::animate_bool(ui.ctx(), resp.id.with("on"), *on, 0.16);
    let b = Rect::from_min_size(Pos2::new(rect.left(), rect.center().y - box_size / 2.0), Vec2::splat(box_size));
    ui.painter().rect_filled(b, theme::RADIUS, theme::CHECK_BG.lerp_to_gamma(theme::CHECK_BG_HOVER, hover));
    if t > 0.01 {
        let p = |x: f32, y: f32| Pos2::new(b.left() + x * box_size, b.top() + y * box_size);
        let color = Color32::WHITE.gamma_multiply(t);
        ui.painter()
            .add(egui::Shape::line(vec![p(0.24, 0.52), p(0.43, 0.70), p(0.78, 0.30)], egui::Stroke::new(2.0, color)));
    }
    let color = theme::BTN_TEXT.lerp_to_gamma(Color32::WHITE, hover);
    ui.painter().galley_with_override_text_color(
        Pos2::new(b.right() + 8.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A Steam `DialogSlider`: a dark track with a blue fill and a round grabber. `width` is the track length.
/// The value is changed in place (rounded for integers); the response reports `changed()` and drag state
/// like any egui widget.
pub fn slider<N: egui::emath::Numeric>(
    ui: &mut Ui,
    value: &mut N,
    range: std::ops::RangeInclusive<N>,
    width: f32,
) -> Response {
    let (lo, hi) = (range.start().to_f64(), range.end().to_f64());
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::click_and_drag());
    let span = (rect.width() - 14.0).max(1.0);
    if (resp.dragged() || resp.clicked() || resp.is_pointer_button_down_on())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let f = ((pos.x - rect.left() - 7.0) / span).clamp(0.0, 1.0) as f64;
        let mut v = lo + (hi - lo) * f;
        if N::INTEGRAL {
            v = v.round();
        }
        let new = N::from_f64(v);
        if new != *value {
            *value = new;
            resp.mark_changed();
        }
    }
    let f = if hi > lo { ((value.to_f64() - lo) / (hi - lo)).clamp(0.0, 1.0) as f32 } else { 0.0 };
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered() || resp.dragged(), anim::FAST);
    // Track: 4 px border, 10 px inside.
    ui.painter().rect_filled(rect, 9.0, theme::SLIDER_BORDER);
    let inner = rect.shrink(4.0);
    ui.painter().rect_filled(inner, 5.0, theme::SLIDER_TRACK);
    let x = rect.left() + 7.0 + span * f;
    let fill = Rect::from_min_max(inner.min, Pos2::new(x.max(inner.left() + 1.0), inner.max.y));
    ui.painter().rect_filled(fill, 5.0, theme::SLIDER_FILL);
    let c = Pos2::new(x, rect.center().y);
    ui.painter().circle_filled(c, 7.5, Color32::from_black_alpha(50));
    let grab = theme::SLIDER_GRAB.lerp_to_gamma(Color32::WHITE, 0.5 * hover);
    ui.painter().circle_filled(c, 6.0, grab);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A single-line text field in Steam's `DialogInput` look: dark fill, 32 px high, 8 px padding, a blue
/// outline while focused. `width` is the full width of the field.
pub fn text_input(ui: &mut Ui, text: &mut String, width: f32, hint: &str) -> Response {
    ui.add(
        egui::TextEdit::singleline(text)
            .desired_width(width)
            .hint_text(hint)
            .margin(egui::Margin::symmetric(theme::INPUT_PAD as i8, 6))
            .font(theme::regular(theme::BTN_FONT)),
    )
}

/// Vertical room above and below each settings row (Steam spaces fields about 12 px apart).
const ROW_PAD: f32 = 9.0;
/// Where the control column of a settings row starts, as a share of the row (labels get the rest).
const LABEL_SHARE: f32 = 0.58;

/// Text of a settings row: label (white) over an optional description (grey), wrapped to `width`.
fn row_text(
    ui: &Ui,
    label: &str,
    desc: &str,
    width: f32,
) -> (std::sync::Arc<egui::Galley>, Option<std::sync::Arc<egui::Galley>>) {
    let name = ui.painter().layout(label.to_string(), theme::regular(14.0), theme::BTN_TEXT, width);
    let note = (!desc.is_empty())
        .then(|| ui.painter().layout(desc.to_string(), theme::regular(13.0), theme::DIALOG_DESC, width));
    (name, note)
}

/// A settings row: label and description on the left, the control on the right (right-aligned, vertically
/// centred). Rows have no lines between them; groups are separated by [`section_header`].
pub fn setting_row(ui: &mut Ui, label: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let text_w = (width * LABEL_SHARE).max(160.0);
    let (name, note) = row_text(ui, label, desc, text_w);
    let text_h = name.size().y + note.as_ref().map_or(0.0, |n| n.size().y + 3.0);
    let row_h = text_h.max(theme::BTN_H) + 2.0 * ROW_PAD;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::hover());
    let top = rect.top() + (row_h - text_h) / 2.0;
    ui.painter().galley(Pos2::new(rect.left(), top), name.clone(), theme::BTN_TEXT);
    if let Some(n) = note {
        ui.painter().galley(Pos2::new(rect.left(), top + name.size().y + 3.0), n, theme::DIALOG_DESC);
    }
    let control_rect = Rect::from_min_max(Pos2::new(rect.left() + text_w + 12.0, rect.top()), rect.right_bottom());
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(control_rect).layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    control(&mut child);
}

/// A settings row whose control is wide (a text field, a list): label and description on top, the control
/// below at full width.
pub fn setting_block(ui: &mut Ui, label: &str, desc: &str, control: impl FnOnce(&mut Ui)) {
    ui.add_space(ROW_PAD);
    ui.label(egui::RichText::new(label).font(theme::regular(14.0)).color(theme::BTN_TEXT));
    if !desc.is_empty() {
        ui.add_space(3.0);
        ui.label(egui::RichText::new(desc).font(theme::regular(13.0)).color(theme::DIALOG_DESC));
    }
    ui.add_space(10.0);
    control(ui);
    ui.add_space(ROW_PAD);
}

/// A group heading inside a settings page (`DialogControlsSectionHeader`): 18 px medium white text with a
/// hairline underneath and 24 px of room above.
pub fn section_header(ui: &mut Ui, text: &str) {
    ui.add_space(24.0);
    let galley =
        ui.painter().layout_no_wrap(text.to_string(), theme::medium(theme::SECTION_HEADER_SIZE), Color32::WHITE);
    let h = galley.size().y + 24.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    ui.painter().galley(Pos2::new(rect.left(), rect.top() + 12.0), galley, Color32::WHITE);
    ui.painter().hline(rect.x_range(), rect.bottom().round() - 0.5, egui::Stroke::new(1.0, theme::SETTINGS_SEP));
    ui.add_space(4.0);
}

/// A small upper-case caption above a field in a dialog (`DialogLabel`: 13 px, #acb2b8).
pub fn field_label(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(text.to_uppercase()).font(theme::regular(13.0)).color(theme::DIALOG_LABEL));
    ui.add_space(4.0);
}

/// The big white title at the top of a settings page or dialog (`DialogHeader`: 22 px bold).
pub fn page_title(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(theme::bold(theme::DIALOG_HEADER_SIZE)).color(Color32::WHITE));
    ui.add_space(6.0);
}
