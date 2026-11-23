//! Tool icons drawn as flat vector shapes with the painter (our own shapes, one colour plus a knock-out colour).
use egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use std::f32::consts::PI;

fn st(w: f32, c: Color32) -> Stroke {
    Stroke::new(w, c)
}

/// Disc with an "i" (game overview).
pub fn info(p: &Painter, r: Rect, c: Color32, knock: Color32) {
    let (m, s) = (r.center(), r.width());
    p.circle_filled(m, s * 0.5, c);
    p.circle_filled(pos2(m.x, m.y - s * 0.2), s * 0.08, knock);
    p.rect_filled(Rect::from_center_size(pos2(m.x, m.y + s * 0.08), vec2(s * 0.15, s * 0.4)), 0.0, knock);
}

/// Medal: disc with a ring and two ribbon tails (achievements).
pub fn medal(p: &Painter, r: Rect, c: Color32, knock: Color32) {
    let (m, s) = (r.center(), r.width());
    let o = pos2(m.x, m.y - s * 0.1);
    for sx in [-1.0f32, 1.0] {
        p.add(Shape::convex_polygon(
            vec![
                pos2(m.x + sx * s * 0.24, m.y + s * 0.1),
                pos2(m.x + sx * s * 0.38, m.y + s * 0.5),
                pos2(m.x + sx * s * 0.12, m.y + s * 0.38),
                pos2(m.x, m.y + s * 0.5),
                pos2(m.x, m.y + s * 0.1),
            ],
            c,
            Stroke::NONE,
        ));
    }
    p.circle_filled(o, s * 0.36, c);
    p.circle_stroke(o, s * 0.22, st(s * 0.07, knock));
    p.circle_filled(o, s * 0.08, knock);
}

/// Pencil (notes).
pub fn pencil(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center(), r.width());
    p.line_segment([m + vec2(-s * 0.2, s * 0.2), m + vec2(s * 0.26, -s * 0.26)], st(s * 0.2, c));
    p.add(Shape::convex_polygon(
        vec![m + vec2(-s * 0.44, s * 0.44), m + vec2(-s * 0.34, s * 0.1), m + vec2(-s * 0.1, s * 0.34)],
        c,
        Stroke::NONE,
    ));
}

/// Book with a bookmark (guides).
pub fn book(p: &Painter, r: Rect, c: Color32, knock: Color32) {
    let (m, s) = (r.center(), r.width());
    let body = Rect::from_center_size(m, vec2(s * 0.62, s * 0.8));
    p.rect_filled(body, s * 0.05, c);
    p.rect_filled(Rect::from_min_size(pos2(body.min.x + s * 0.12, body.min.y), vec2(s * 0.1, s * 0.4)), 0.0, knock);
    p.rect_filled(
        Rect::from_min_size(pos2(body.min.x + s * 0.3, body.min.y + s * 0.16), vec2(s * 0.22, s * 0.05)),
        0.0,
        knock,
    );
    p.rect_filled(
        Rect::from_min_size(pos2(body.min.x + s * 0.3, body.min.y + s * 0.28), vec2(s * 0.22, s * 0.05)),
        0.0,
        knock,
    );
}

/// Corner brackets around a dot (screenshots).
pub fn screenshot(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center(), r.width());
    let (h, l) = (s * 0.4, s * 0.22);
    let w = st(s * 0.1, c);
    for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let corner = pos2(m.x + sx * h, m.y + sy * h);
        p.line_segment([corner, pos2(corner.x - sx * l, corner.y)], w);
        p.line_segment([corner, pos2(corner.x, corner.y - sy * l)], w);
    }
    p.circle_filled(m, s * 0.15, c);
}

/// Two people (friends).
pub fn people(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center(), r.width());
    for (dx, sc) in [(s * 0.14, 1.0f32), (-s * 0.2, 0.85)] {
        let cx = m.x + dx;
        p.circle_filled(pos2(cx, m.y - s * 0.16 * sc), s * 0.15 * sc, c);
        let sh =
            Rect::from_min_max(pos2(cx - s * 0.24 * sc, m.y + s * 0.08 * sc), pos2(cx + s * 0.24 * sc, m.y + s * 0.34));
        p.rect_filled(sh, CornerRadius { nw: 9, ne: 9, sw: 1, se: 1 }, c);
    }
}

/// Compass (web browser).
pub fn compass(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center(), r.width());
    p.circle_stroke(m, s * 0.42, st(s * 0.1, c));
    let u = vec2(0.7, -0.7);
    let v = vec2(0.7, 0.7);
    p.add(Shape::convex_polygon(
        vec![m + u * s * 0.3, m + v * s * 0.09, m - u * s * 0.3, m - v * s * 0.09],
        c,
        Stroke::NONE,
    ));
}

/// Gear.
pub fn gear(p: &Painter, r: Rect, c: Color32, knock: Color32) {
    let (m, s) = (r.center(), r.width());
    for i in 0..8 {
        let a = i as f32 * PI / 4.0;
        let d = vec2(a.cos(), a.sin());
        p.line_segment([m + d * s * 0.3, m + d * s * 0.46], st(s * 0.17, c));
    }
    p.circle_filled(m, s * 0.37, c);
    p.circle_filled(m, s * 0.15, knock);
}

/// Speedometer (performance monitor).
pub fn gauge(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center() + vec2(0.0, r.width() * 0.05), r.width());
    let n = 24;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = PI * (0.85 + 1.3 * i as f32 / n as f32);
            m + vec2(a.cos(), a.sin()) * s * 0.4
        })
        .collect();
    p.add(Shape::line(pts, st(s * 0.1, c)));
    p.line_segment([m, m + vec2(0.26, -0.26) * s], st(s * 0.1, c));
    p.circle_filled(m, s * 0.09, c);
}

/// Close cross; `r` is the box of the cross.
pub fn cross(p: &Painter, r: Rect, c: Color32, w: f32) {
    let h = r.width() * 0.5;
    let m = r.center();
    p.line_segment([m + vec2(-h, -h), m + vec2(h, h)], st(w, c));
    p.line_segment([m + vec2(-h, h), m + vec2(h, -h)], st(w, c));
}

/// Magnifier.
pub fn search(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center(), r.width());
    let o = m + vec2(-s * 0.1, -s * 0.1);
    p.circle_stroke(o, s * 0.28, st(s * 0.12, c));
    p.line_segment([o + vec2(0.2, 0.2) * s, o + vec2(0.46, 0.46) * s], st(s * 0.14, c));
}

/// Trophy.
pub fn trophy(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center(), r.width());
    p.rect_filled(
        Rect::from_min_max(pos2(m.x - s * 0.28, m.y - s * 0.42), pos2(m.x + s * 0.28, m.y + s * 0.05)),
        CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 },
        c,
    );
    p.rect_filled(Rect::from_center_size(pos2(m.x, m.y + s * 0.22), vec2(s * 0.12, s * 0.3)), 0.0, c);
    p.rect_filled(Rect::from_center_size(pos2(m.x, m.y + s * 0.42), vec2(s * 0.5, s * 0.1)), 0.0, c);
}

/// Three horizontal lines (section handle, list rows).
pub fn grip(p: &Painter, r: Rect, c: Color32) {
    let m = r.center();
    let w = r.width() * 0.5;
    for dy in [-0.28f32, 0.0, 0.28] {
        p.line_segment(
            [pos2(m.x - w, m.y + dy * r.height()), pos2(m.x + w, m.y + dy * r.height())],
            st(r.height() * 0.12, c),
        );
    }
}

/// Small down chevron.
pub fn chevron(p: &Painter, c: Pos2, s: f32, col: Color32) {
    p.line_segment([c + vec2(-s, -s * 0.5), c + vec2(0.0, s * 0.5)], st(s * 0.4, col));
    p.line_segment([c + vec2(0.0, s * 0.5), c + vec2(s, -s * 0.5)], st(s * 0.4, col));
}

/// Clock face (playtime).
pub fn clock(p: &Painter, r: Rect, c: Color32) {
    let (m, s) = (r.center(), r.width());
    p.circle_filled(m, s * 0.5, c);
    let k = Color32::from_rgb(0x3d, 0x43, 0x4f);
    p.line_segment([m, m + vec2(0.0, -s * 0.3)], st(s * 0.1, k));
    p.line_segment([m, m + vec2(s * 0.22, s * 0.1)], st(s * 0.1, k));
}

/// Small book on a section bar (guides).
pub fn book_small(p: &Painter, r: Rect, c: Color32) {
    book(p, r, c, Color32::from_rgb(0x3d, 0x43, 0x4f));
}
