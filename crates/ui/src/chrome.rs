//! Window chrome: our own title bar controls, window dragging and edge resize.
//! The window has no OS decorations (`with_decorations(false)`), so everything
//! the window manager would normally give us is drawn and handled here.

use crate::{anim, theme};
use egui::{Context, CursorIcon, Id, Rect, Sense, Ui, Vec2, ViewportCommand, viewport::ResizeDirection};
use egui_phosphor::regular as icon;

fn is_maximized(ctx: &Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false) || i.viewport().fullscreen.unwrap_or(false))
}

pub fn toggle_maximize(ctx: &Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized(ctx)));
}

/// Make the whole of `ui`'s area a window drag handle. Call it first, before
/// adding widgets: widgets added later sit on top and keep their own clicks.
pub fn drag_region(ui: &Ui) {
    let resp = ui.interact(ui.max_rect(), ui.id().with("drag-region"), Sense::click_and_drag());
    if resp.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
    if resp.double_clicked() {
        toggle_maximize(ui.ctx());
    }
}

/// Minimize, maximize/restore and close, right-aligned (call in a right-to-left layout).
pub fn window_buttons(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    ui.spacing_mut().item_spacing.x = 0.0;
    let maximized = is_maximized(&ctx);
    if window_button(ui, icon::X, true).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Close);
    }
    if window_button(ui, if maximized { icon::COPY } else { icon::SQUARE }, false).clicked() {
        toggle_maximize(&ctx);
    }
    if window_button(ui, icon::MINUS, false).clicked() {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
    }
}

fn window_button(ui: &mut Ui, glyph: &str, close: bool) -> egui::Response {
    let size = Vec2::new(theme::WINDOW_BUTTON_W, theme::MENU_BAR_H);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let t = anim::animate_bool(ui.ctx(), resp.id.with("hover"), resp.hovered(), anim::FAST);
    if t > 0.0 {
        let bg = if close { theme::CLOSE_HOVER } else { theme::BG_ROW_SELECTED };
        ui.painter().rect_filled(rect, 0.0, bg.gamma_multiply(t));
    }
    let color = theme::TEXT_HEADING.lerp_to_gamma(egui::Color32::WHITE, t);
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, glyph, theme::regular(15.0), color);
    resp
}

/// Alt + left-drag anywhere moves the window, like many Linux desktops do.
/// While Alt is held a full-window layer sits on top so the press never
/// reaches the widgets below (no accidental clicks).
pub fn alt_drag(ui: &mut Ui) {
    if !ui.input(|i| i.modifiers.alt) {
        return;
    }
    let rect = ui.ctx().content_rect();
    let resp = ui.interact(rect, Id::new("alt-drag"), Sense::click_and_drag());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::Grab);
    }
    if ui.input(|i| i.pointer.primary_pressed()) && resp.hovered() {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// Invisible edge and corner zones that start a resize. Call last so they win
/// over whatever is drawn at the window edge. Not shown when maximized.
pub fn resize_zones(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    paint_border(ui, ctx.content_rect());
    if is_maximized(&ctx) {
        return;
    }
    let r = ctx.content_rect();
    let (e, c) = (theme::RESIZE_EDGE, theme::RESIZE_EDGE * 2.0);
    let zone =
        |min: (f32, f32), max: (f32, f32)| Rect::from_min_max(egui::pos2(min.0, min.1), egui::pos2(max.0, max.1));
    let zones = [
        (zone((r.left() + c, r.top()), (r.right() - c, r.top() + e)), ResizeDirection::North, CursorIcon::ResizeNorth),
        (
            zone((r.left() + c, r.bottom() - e), (r.right() - c, r.bottom())),
            ResizeDirection::South,
            CursorIcon::ResizeSouth,
        ),
        (zone((r.left(), r.top() + c), (r.left() + e, r.bottom() - c)), ResizeDirection::West, CursorIcon::ResizeWest),
        (
            zone((r.right() - e, r.top() + c), (r.right(), r.bottom() - c)),
            ResizeDirection::East,
            CursorIcon::ResizeEast,
        ),
        (
            zone((r.left(), r.top()), (r.left() + c, r.top() + c)),
            ResizeDirection::NorthWest,
            CursorIcon::ResizeNorthWest,
        ),
        (
            zone((r.right() - c, r.top()), (r.right(), r.top() + c)),
            ResizeDirection::NorthEast,
            CursorIcon::ResizeNorthEast,
        ),
        (
            zone((r.left(), r.bottom() - c), (r.left() + c, r.bottom())),
            ResizeDirection::SouthWest,
            CursorIcon::ResizeSouthWest,
        ),
        (
            zone((r.right() - c, r.bottom() - c), (r.right(), r.bottom())),
            ResizeDirection::SouthEast,
            CursorIcon::ResizeSouthEast,
        ),
    ];
    for (i, (rect, dir, cursor)) in zones.into_iter().enumerate() {
        let resp = ui.interact(rect, Id::new(("resize", i)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

fn paint_border(ui: &Ui, rect: Rect) {
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, 0.0, (1.0, theme::WINDOW_BORDER), egui::StrokeKind::Inside);
}
