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

/// Fill `rect` with a top-to-bottom gradient.
pub fn vgradient(ui: &Ui, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = Mesh::default();
    for (pos, c) in
        [(rect.left_top(), top), (rect.right_top(), top), (rect.right_bottom(), bottom), (rect.left_bottom(), bottom)]
    {
        mesh.colored_vertex(pos, c);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    ui.painter().add(mesh);
}

/// The big Play / Install button: gradient, icon + bold label.
pub fn action_button(ui: &mut Ui, icon: &str, label: &str, colors: (Color32, Color32)) -> Response {
    let font = theme::bold(16.0);
    let text = format!("{icon}  {label}");
    let galley = ui.painter().layout_no_wrap(text, font, Color32::WHITE);
    let size = Vec2::new((galley.size().x + 56.0).max(150.0), 46.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    // Brighten on hover, darken and shrink a little while pressed.
    let lift = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    let press = anim::animate_bool(ui.ctx(), resp.id.with("press"), resp.is_pointer_button_down_on(), 0.08);
    let k = 1.0 + 0.15 * lift - 0.18 * press;
    let rect = rect.shrink(1.5 * press);
    gradient(ui, rect, colors.0.gamma_multiply(k), colors.1.gamma_multiply(k));
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A square icon button (gear, info, ...) that lightens on hover.
pub fn icon_button(ui: &mut Ui, icon: &str, size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let t = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    let press = anim::animate_bool(ui.ctx(), resp.id.with("press"), resp.is_pointer_button_down_on(), 0.08);
    if t > 0.0 {
        ui.painter().rect_filled(
            rect,
            theme::BUTTON_RADIUS,
            theme::BG_ROW_SELECTED.gamma_multiply(t * (1.0 - 0.3 * press)),
        );
    }
    let color = theme::TEXT_DIM.lerp_to_gamma(theme::TEXT, t);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, theme::regular(size * 0.55), color);
    resp.on_hover_text(tooltip)
}

/// A text label that can be clicked, brightening on hover (nav tabs, links).
/// The active-tab underline is drawn by the caller so it can slide.
pub fn text_tab(ui: &mut Ui, text: &str, font: egui::FontId, active: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, theme::TEXT);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(0.0, 4.0), Sense::click());
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered() && !active, anim::FAST);
    let on = anim::animate_bool(ui.ctx(), resp.id.with("on"), active, anim::NORMAL);
    let color = theme::TEXT.lerp_to_gamma(Color32::WHITE, hover).lerp_to_gamma(theme::ACCENT, on);
    ui.painter().galley_with_override_text_color(Pos2::new(rect.left(), rect.top() + 2.0), galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    /// Blue, for the main action of a dialog.
    Primary,
    /// Grey, for everything else.
    Secondary,
    /// Red, for destructive actions.
    Danger,
}

/// A flat text button with a hover fade. Returns its response; `enabled == false` draws it dim and inert.
pub fn button(ui: &mut Ui, label: &str, style: ButtonStyle, enabled: bool) -> Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::medium(13.0), theme::TEXT);
    let size = Vec2::new(galley.size().x + 28.0, 30.0);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    let hover = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered() && enabled, anim::FAST);
    let press = anim::animate_bool(ui.ctx(), resp.id.with("press"), resp.is_pointer_button_down_on(), 0.08);
    let base = match style {
        ButtonStyle::Primary => theme::INSTALL_B,
        ButtonStyle::Secondary => theme::BG_INPUT,
        ButtonStyle::Danger => theme::DANGER,
    };
    let lit = base.lerp_to_gamma(Color32::WHITE, 0.12);
    let mut bg = base.lerp_to_gamma(lit, hover).gamma_multiply(1.0 - 0.15 * press);
    let mut fg = Color32::WHITE;
    if !enabled {
        bg = theme::BG_INPUT.gamma_multiply(0.6);
        fg = theme::TEXT_UNINSTALLED;
    }
    ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, bg);
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, fg);
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
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
