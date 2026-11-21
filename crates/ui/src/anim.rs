//! Tiny animation helpers. Animations are short (120-220 ms) and ease out.
//! Idle CPU stays at zero because a repaint is requested only while a value
//! is actually moving.

use egui::{Context, Id};
pub use sr_client::AnimationMode;

/// Remember the animation setting for this frame (call once per frame, before drawing).
pub fn set_mode(ctx: &Context, mode: AnimationMode) {
    ctx.data_mut(|d| d.insert_temp(Id::new("anim-mode"), mode));
}

pub fn mode(ctx: &Context) -> AnimationMode {
    ctx.data(|d| d.get_temp::<AnimationMode>(Id::new("anim-mode"))).unwrap_or_default()
}

/// False when animations are off: everything jumps to its end value.
pub fn enabled(ctx: &Context) -> bool {
    mode(ctx) != AnimationMode::Off
}

/// True only for "full": things may move (slide, lift, scale). "Reduced" keeps fades only.
pub fn movement(ctx: &Context) -> bool {
    mode(ctx) == AnimationMode::Full
}

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
    if !enabled(ctx) {
        return target;
    }
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
    if !enabled(ctx) {
        return 1.0;
    }
    let t = ((ctx.input(|i| i.time) - start) as f32 / dur).clamp(0.0, 1.0);
    if t < 1.0 {
        ctx.request_repaint();
    }
    ease_out(t)
}

/// Move a stored value towards `target` at a steady rate (`per_sec` of the distance per second, 0..1 fractions
/// feel smooth around 3.0). Meant for progress bars that get a new number a few times a second: the bar glides
/// between updates instead of jumping. Repaints at about 15 Hz, and only while the value is still moving, so a
/// bar that is full or paused costs nothing. The first call for an id returns `target` at once.
pub fn glide(ctx: &Context, id: Id, target: f32, per_sec: f32) -> f32 {
    if !enabled(ctx) {
        return target;
    }
    #[derive(Clone, Copy)]
    struct Glide {
        value: f32,
        at: f64,
    }
    let now = ctx.input(|i| i.time);
    let Some(g) = ctx.data(|d| d.get_temp::<Glide>(id)) else {
        ctx.data_mut(|d| d.insert_temp(id, Glide { value: target, at: now }));
        return target;
    };
    let dt = (now - g.at).max(0.0) as f32;
    let step = (target - g.value) * (1.0 - (-per_sec * dt).exp());
    let value = if (target - g.value).abs() < 0.0005 { target } else { g.value + step };
    ctx.data_mut(|d| d.insert_temp(id, Glide { value, at: now }));
    if (target - value).abs() >= 0.0005 {
        ctx.request_repaint_after(std::time::Duration::from_millis(66));
    }
    value
}
