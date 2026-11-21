//! Our own vector drawings of the shapes the official client uses for its small icons. Every icon is drawn
//! with the painter (lines, circles, polygons) around a centre point at a given size in points, so it stays
//! sharp at any zoom. The silhouettes follow what the client shows; the code and numbers are ours.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2, pos2, vec2};

fn line(p: &Painter, pts: &[Pos2], w: f32, c: Color32) {
    p.add(Shape::line(pts.to_vec(), Stroke::new(w, c)));
}

fn fill(p: &Painter, pts: &[Pos2], c: Color32) {
    p.add(Shape::convex_polygon(pts.to_vec(), c, Stroke::NONE));
}

/// Small down-pointing chevron (drop-downs).
pub fn chevron_down(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let (w, h) = (size * 0.5, size * 0.25);
    line(p, &[c + vec2(-w, -h), c + vec2(0.0, h), c + vec2(w, -h)], 1.4, color);
}

/// Left or right arrow of the history buttons: a shaft with an open head.
pub fn arrow(p: &Painter, c: Pos2, size: f32, left: bool, color: Color32) {
    let s = if left { -1.0 } else { 1.0 };
    let half = size * 0.5;
    let tip = c + vec2(s * half, 0.0);
    let tail = c + vec2(-s * half, 0.0);
    line(p, &[tail, tip], 2.6, color);
    let head = size * 0.45;
    line(p, &[tip + vec2(-s * head, -head), tip, tip + vec2(-s * head, head)], 2.6, color);
}

/// Up arrow of the sort button.
pub fn arrow_up(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let half = size * 0.5;
    line(p, &[c + vec2(0.0, half), c + vec2(0.0, -half)], 1.6, color);
    let head = size * 0.32;
    line(p, &[c + vec2(-head, -half + head), c + vec2(0.0, -half), c + vec2(head, -half + head)], 1.6, color);
}

/// Magnifying glass: a ring with a handle to the lower right.
pub fn magnifier(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let r = size * 0.34;
    let ring = c + vec2(-size * 0.1, -size * 0.1);
    p.circle_stroke(ring, r, Stroke::new(1.5, color));
    let from = ring + vec2(r, r) * 0.72;
    line(p, &[from, c + vec2(size * 0.46, size * 0.46)], 1.8, color);
}

/// Filter button: three bars getting shorter, centred.
pub fn filter_lines(p: &Painter, c: Pos2, size: f32, color: Color32) {
    for (i, w) in [1.0_f32, 0.66, 0.34].into_iter().enumerate() {
        let y = c.y + (i as f32 - 1.0) * size * 0.34;
        let hw = size * 0.5 * w;
        line(p, &[pos2(c.x - hw, y), pos2(c.x + hw, y)], 1.4, color);
    }
}

/// 2 x 2 grid of squares (the grid view button).
pub fn grid(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let cell = size * 0.42;
    let gap = size * 0.16;
    for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let mid = c + vec2(dx * (cell + gap) / 2.0, dy * (cell + gap) / 2.0);
        p.rect_filled(Rect::from_center_size(mid, Vec2::splat(cell)), 1.0, color);
    }
}

/// Clock: a ring with two hands.
pub fn clock(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let r = size * 0.5;
    p.circle_stroke(c, r, Stroke::new(1.3, color));
    line(p, &[c + vec2(0.0, -r * 0.6), c, c + vec2(r * 0.5, r * 0.35)], 1.3, color);
}

/// Ring with a play triangle inside.
pub fn play_circle(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let r = size * 0.5;
    p.circle_stroke(c, r, Stroke::new(1.3, color));
    let t = r * 0.5;
    fill(p, &[c + vec2(-t * 0.6, -t), c + vec2(t, 0.0), c + vec2(-t * 0.6, t)], color);
}

/// Solid play triangle (PLAY button).
pub fn play_triangle(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let h = size * 0.5;
    fill(p, &[c + vec2(-h * 0.8, -h), c + vec2(h, 0.0), c + vec2(-h * 0.8, h)], color);
}

/// Gear: a disc with six teeth and a hole in `hole` colour.
pub fn gear(p: &Painter, c: Pos2, size: f32, color: Color32, hole: Color32) {
    let r = size * 0.5;
    for i in 0..6 {
        let a = i as f32 * std::f32::consts::TAU / 6.0;
        let (s, co) = a.sin_cos();
        let dir = vec2(co, s);
        let perp = vec2(-s, co);
        let (w, o, i2) = (r * 0.3, r, r * 0.55);
        fill(
            p,
            &[
                c + dir * i2 + perp * w,
                c + dir * o + perp * (w * 0.8),
                c + dir * o - perp * (w * 0.8),
                c + dir * i2 - perp * w,
            ],
            color,
        );
    }
    p.circle_filled(c, r * 0.72, color);
    p.circle_filled(c, r * 0.28, hole);
}

/// Filled circle with an "i" cut out in `cut` colour.
pub fn info(p: &Painter, c: Pos2, size: f32, color: Color32, cut: Color32) {
    let r = size * 0.5;
    p.circle_filled(c, r, color);
    p.circle_filled(c + vec2(0.0, -r * 0.45), r * 0.14, cut);
    p.rect_filled(Rect::from_center_size(c + vec2(0.0, r * 0.15), vec2(r * 0.26, r * 0.7)), 0.5, cut);
}

/// Heart: two round lobes and a point.
pub fn heart(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.5;
    let lobe = s * 0.52;
    p.circle_filled(c + vec2(-lobe * 0.95, -s * 0.28), lobe, color);
    p.circle_filled(c + vec2(lobe * 0.95, -s * 0.28), lobe, color);
    fill(p, &[c + vec2(-s * 0.98, -s * 0.12), c + vec2(s * 0.98, -s * 0.12), c + vec2(0.0, s * 0.95)], color);
}

/// Bell: a dome, a flared body, a rim and a clapper.
pub fn bell(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.5;
    p.circle_filled(c + vec2(0.0, -s * 0.2), s * 0.55, color);
    fill(
        p,
        &[
            c + vec2(-s * 0.55, -s * 0.2),
            c + vec2(s * 0.55, -s * 0.2),
            c + vec2(s * 0.85, s * 0.5),
            c + vec2(-s * 0.85, s * 0.5),
        ],
        color,
    );
    p.circle_filled(c + vec2(0.0, s * 0.8), s * 0.22, color);
    p.rect_filled(Rect::from_center_size(c + vec2(0.0, -s * 0.78), vec2(s * 0.2, s * 0.2)), 1.0, color);
}

/// Megaphone: a cone that opens to the right, with a back box and a handle.
pub fn megaphone(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.5;
    p.rect_filled(Rect::from_min_max(c + vec2(-s, -s * 0.32), c + vec2(-s * 0.35, s * 0.32)), 1.0, color);
    fill(
        p,
        &[
            c + vec2(-s * 0.4, -s * 0.32),
            c + vec2(s * 0.7, -s * 0.9),
            c + vec2(s * 0.7, s * 0.9),
            c + vec2(-s * 0.4, s * 0.32),
        ],
        color,
    );
    p.rect_filled(Rect::from_min_max(c + vec2(s * 0.7, -s * 0.2), c + vec2(s, s * 0.2)), 1.0, color);
    p.rect_filled(Rect::from_min_max(c + vec2(-s * 0.85, s * 0.3), c + vec2(-s * 0.5, s * 0.95)), 1.0, color);
}

/// Monitor with a stand (the "show on screen" button).
pub fn monitor(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.5;
    let screen = Rect::from_center_size(c + vec2(0.0, -s * 0.12), vec2(size, size * 0.66));
    p.rect_filled(screen, 2.0, color);
    p.rect_filled(Rect::from_center_size(c + vec2(0.0, s * 0.74), vec2(size * 0.5, 2.0)), 0.5, color);
}

/// Window controls: minimise, maximise (a box with a heavier top edge), restore, close.
pub fn window_minimize(p: &Painter, c: Pos2, color: Color32) {
    line(p, &[c + vec2(-7.0, 0.0), c + vec2(7.0, 0.0)], 1.4, color);
}

pub fn window_maximize(p: &Painter, c: Pos2, color: Color32) {
    let r = Rect::from_center_size(c, vec2(14.0, 12.0));
    p.rect_stroke(r, 0.0, Stroke::new(1.3, color), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(r.min, vec2(14.0, 3.0)), 0.0, color);
}

pub fn window_restore(p: &Painter, c: Pos2, color: Color32, bg: Color32) {
    let back = Rect::from_center_size(c + vec2(2.0, -2.0), vec2(11.0, 10.0));
    let front = Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(11.0, 10.0));
    p.rect_stroke(back, 0.0, Stroke::new(1.2, color), egui::StrokeKind::Inside);
    p.rect_filled(front, 0.0, bg);
    p.rect_stroke(front, 0.0, Stroke::new(1.2, color), egui::StrokeKind::Inside);
    p.rect_filled(Rect::from_min_size(front.min, vec2(11.0, 2.5)), 0.0, color);
}

pub fn window_close(p: &Painter, c: Pos2, color: Color32) {
    let d = 6.0;
    line(p, &[c + vec2(-d, -d), c + vec2(d, d)], 1.4, color);
    line(p, &[c + vec2(-d, d), c + vec2(d, -d)], 1.4, color);
}

/// Square with a plus (Add a Game).
pub fn add_square(p: &Painter, c: Pos2, size: f32, color: Color32, cut: Color32) {
    p.rect_filled(Rect::from_center_size(c, Vec2::splat(size)), 2.0, color);
    let a = size * 0.26;
    line(p, &[c + vec2(-a, 0.0), c + vec2(a, 0.0)], 1.6, cut);
    line(p, &[c + vec2(0.0, -a), c + vec2(0.0, a)], 1.6, cut);
}

/// Square with a down arrow (Downloads).
pub fn download_square(p: &Painter, c: Pos2, size: f32, color: Color32, cut: Color32) {
    p.rect_filled(Rect::from_center_size(c, Vec2::splat(size)), 2.0, color);
    let a = size * 0.24;
    line(p, &[c + vec2(0.0, -a), c + vec2(0.0, a * 0.8)], 1.6, cut);
    line(p, &[c + vec2(-a, 0.0), c + vec2(0.0, a * 0.9), c + vec2(a, 0.0)], 1.6, cut);
    line(p, &[c + vec2(-a, a * 1.4), c + vec2(a, a * 1.4)], 1.6, cut);
}

/// Friends & Chat: a square with two notches cut out in `cut` colour.
pub fn friends_square(p: &Painter, c: Pos2, size: f32, color: Color32, cut: Color32) {
    p.rect_filled(Rect::from_center_size(c, Vec2::splat(size)), 2.0, color);
    for d in [vec2(size * 0.17, -size * 0.17), vec2(-size * 0.17, size * 0.17)] {
        p.rect_filled(Rect::from_center_size(c + d, Vec2::splat(size * 0.3)), 1.0, cut);
    }
}

/// Three dots (the "..." of the tab box).
pub fn ellipsis(p: &Painter, c: Pos2, color: Color32) {
    for dx in [-5.0, 0.0, 5.0] {
        p.circle_filled(c + vec2(dx, 0.0), 1.5, color);
    }
}

/// Lightning bolt (dynamic collection badge).
pub fn bolt(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.5;
    fill(
        p,
        &[
            c + vec2(s * 0.2, -s),
            c + vec2(-s * 0.6, s * 0.1),
            c + vec2(-s * 0.05, s * 0.1),
            c + vec2(-s * 0.2, s),
            c + vec2(s * 0.6, -s * 0.1),
            c + vec2(s * 0.05, -s * 0.1),
        ],
        color,
    );
}

/// The two-part arrow of the PLAY button's drop-down segment: a bar over a small down triangle.
pub fn play_dropdown(p: &Painter, c: Pos2, color: Color32) {
    p.rect_filled(Rect::from_center_size(c + vec2(0.0, -3.0), vec2(8.0, 1.6)), 0.0, color);
    fill(p, &[c + vec2(-4.0, -0.5), c + vec2(4.0, -0.5), c + vec2(0.0, 3.5)], color);
}

/// Kettle's own mark: a kettle seen from the side (replaces the official logo).
pub fn kettle_mark(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let s = size * 0.5;
    p.add(Shape::ellipse_filled(c + vec2(-s * 0.1, s * 0.2), vec2(s * 0.7, s * 0.65), color));
    p.rect_filled(Rect::from_center_size(c + vec2(-s * 0.1, -s * 0.5), vec2(s * 0.5, s * 0.16)), 1.0, color);
    p.circle_filled(c + vec2(-s * 0.1, -s * 0.68), s * 0.12, color);
    fill(
        p,
        &[
            c + vec2(s * 0.45, s * 0.1),
            c + vec2(s * 0.95, -s * 0.55),
            c + vec2(s * 0.95, -s * 0.3),
            c + vec2(s * 0.55, s * 0.4),
        ],
        color,
    );
    line(
        p,
        &[
            c + vec2(-s * 0.65, -s * 0.1),
            c + vec2(-s * 1.0, -s * 0.05),
            c + vec2(-s * 1.0, s * 0.5),
            c + vec2(-s * 0.7, s * 0.5),
        ],
        1.6,
        color,
    );
}

/// Down arrow with a base line (Install / Update).
pub fn arrow_down(p: &Painter, c: Pos2, size: f32, color: Color32) {
    let half = size * 0.5;
    line(p, &[c + vec2(0.0, -half), c + vec2(0.0, half * 0.6)], 2.0, color);
    let head = size * 0.34;
    line(
        p,
        &[c + vec2(-head, half * 0.6 - head), c + vec2(0.0, half * 0.6), c + vec2(head, half * 0.6 - head)],
        2.0,
        color,
    );
    line(p, &[c + vec2(-half * 0.8, half), c + vec2(half * 0.8, half)], 2.0, color);
}
