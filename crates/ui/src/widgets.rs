//! Small reusable widgets drawn with the painter.

use crate::theme;
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
    let lift = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.1);
    let (a, b) = (colors.0.gamma_multiply(1.0 + 0.15 * lift), colors.1.gamma_multiply(1.0 + 0.15 * lift));
    gradient(ui, rect, a, b);
    let pos = rect.center() - galley.size() / 2.0;
    ui.painter().galley(pos, galley, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A square icon button (gear, info, ...) that lightens on hover.
pub fn icon_button(ui: &mut Ui, icon: &str, size: f32, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, theme::BUTTON_RADIUS, theme::BG_ROW_SELECTED);
    }
    let color = if resp.hovered() { theme::TEXT } else { theme::TEXT_DIM };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, theme::regular(size * 0.55), color);
    resp.on_hover_text(tooltip)
}

/// A text label that can be clicked, brightening on hover (nav tabs, links).
pub fn text_tab(ui: &mut Ui, text: &str, font: egui::FontId, active: bool) -> Response {
    let color = if active { theme::ACCENT } else { theme::TEXT };
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(0.0, 4.0), Sense::click());
    let color = if resp.hovered() && !active { Color32::WHITE } else { color };
    ui.painter().galley_with_override_text_color(Pos2::new(rect.left(), rect.top() + 2.0), galley, color);
    if active {
        let y = rect.bottom() + 1.0;
        ui.painter().line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], (2.0, theme::ACCENT));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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
pub fn image_fit_bottom_left(ui: &Ui, bounds: Rect, tex: &egui::TextureHandle) {
    let [w, h] = tex.size().map(|v| v as f32);
    let s = (bounds.width() / w).min(bounds.height() / h);
    let size = Vec2::new(w * s, h * s);
    let rect = Rect::from_min_size(Pos2::new(bounds.left(), bounds.bottom() - size.y), size);
    ui.painter().image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
}
