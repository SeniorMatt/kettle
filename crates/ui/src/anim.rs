//! Tiny animation helpers. Animations are short (120-220 ms) and ease out.
//! Idle CPU stays at zero because a repaint is requested only while a value
//! is actually moving.

use egui::{Context, Id};

/// Default duration of UI transitions, in seconds.
pub const FAST: f32 = 0.14;
pub const NORMAL: f32 = 0.2;

/// Cubic ease-out: quick start, soft landing.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: f64,
}

impl Tween {
    fn at(&self, now: f64, dur: f32) -> f32 {
        let t = ((now - self.start) as f32 / dur).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * ease_out(t)
    }
}

/// Move a value towards `target` with an ease-out curve. A new target restarts
/// the tween from wherever the value currently is. The first call for an id
/// returns `target` right away (no animation on first appearance).
pub fn animate_to(ctx: &Context, id: Id, target: f32, dur: f32) -> f32 {
    let now = ctx.input(|i| i.time);
    let stored = ctx.data(|d| d.get_temp::<Tween>(id));
    let Some(mut tw) = stored else {
        // Nothing stored and nothing to animate: stay out of the map.
        if target != 0.0 {
            ctx.data_mut(|d| d.insert_temp(id, Tween { from: target, to: target, start: now }));
        }
        return target;
    };
    if (tw.to - target).abs() > f32::EPSILON {
        tw = Tween { from: tw.at(now, dur), to: target, start: now };
    }
    let value = tw.at(now, dur);
    let running = now - tw.start < f64::from(dur);
    if running {
        ctx.request_repaint();
    }
    if !running && target == 0.0 {
        ctx.data_mut(|d| d.remove::<Tween>(id));
    } else {
        ctx.data_mut(|d| d.insert_temp(id, tw));
    }
    value
}

/// Animated 0..1 for a boolean (hover, pressed, selected).
pub fn animate_bool(ctx: &Context, id: Id, on: bool, dur: f32) -> f32 {
    animate_to(ctx, id, if on { 1.0 } else { 0.0 }, dur)
}

/// Progress 0..1 (eased) of an animation that started at `start` (egui time).
/// Requests repaints until it is done.
pub fn since(ctx: &Context, start: f64, dur: f32) -> f32 {
    let t = ((ctx.input(|i| i.time) - start) as f32 / dur).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out(t)
}
