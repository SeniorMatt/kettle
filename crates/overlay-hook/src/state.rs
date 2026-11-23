//! The one overlay of this process: input state, the egui context, the data from Kettle, frame timing.
//! The renderers (`gl`, `vk`) ask it for a frame; the input hooks (`sdl`, `xlib`) feed it events.

use crate::input::Input;
use crate::ipc;
use egui::{ClippedPrimitive, Context, Pos2, Rect, TexturesDelta, Vec2};
use sr_overlay_proto::{OverlayAction, OverlayData};
use sr_overlay_ui::{
    OverlayAchievements, OverlayAction as UiAction, OverlayData as UiData, OverlayFriend, OverlayPerf, OverlayState,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

pub struct Core {
    pub input: Input,
    pub ctx: Context,
    pub data: OverlayData,
    pub state: OverlayState,
    start: Instant,
    /// Present times of the last second.
    presents: VecDeque<Instant>,
    worst: Duration,
    worst_since: Instant,
    worst_last: Duration,
    last_present: Option<Instant>,
}

/// What the renderer draws this frame.
pub struct Frame {
    pub primitives: Vec<ClippedPrimitive>,
    pub textures: TexturesDelta,
    pub pixels_per_point: f32,
}

/// The overlay is open (read by the hooks without a lock).
static OPEN: AtomicBool = AtomicBool::new(false);
/// The state the input backends (SDL, Xlib) have been switched to.
static APPLIED: AtomicBool = AtomicBool::new(false);

pub fn is_open() -> bool {
    OPEN.load(Ordering::Acquire)
}

fn core() -> &'static Mutex<Core> {
    static CORE: OnceLock<Mutex<Core>> = OnceLock::new();
    CORE.get_or_init(|| {
        let now = Instant::now();
        let mut data = OverlayData { enabled: true, animations: true, ..Default::default() };
        if std::env::var_os("KETTLE_OVERLAY_DEMO").is_some() {
            data.game = "Demo game".into();
            data.me = "demo".into();
            data.hud = std::env::var("KETTLE_OVERLAY_DEMO").is_ok_and(|v| v == "hud");
        }
        let c = Core {
            input: Input::default(),
            ctx: Context::default(),
            data,
            state: OverlayState::default(),
            start: now,
            presents: VecDeque::new(),
            worst: Duration::ZERO,
            worst_since: now,
            worst_last: Duration::ZERO,
            last_present: None,
        };
        ipc::start();
        Mutex::new(c)
    })
}

/// The overlay's state. Never held across a call into the game or the system.
pub fn lock() -> MutexGuard<'static, Core> {
    core().lock().unwrap_or_else(|e| e.into_inner())
}

/// Open or close from anywhere (a key, Kettle's menu, the Close button). The input backends follow the next
/// time one of their hooks runs, see [`sync_backends`].
pub fn set_open(open: bool) {
    let mut c = lock();
    if c.input.disabled && open {
        return;
    }
    c.input.set_open(open);
    publish(&c);
}

/// Feed the input state machine; afterwards the open state is published and the input backends follow it.
pub fn with_input<R>(f: impl FnOnce(&mut Input) -> R) -> R {
    sync_backends();
    let r = {
        let mut c = lock();
        let r = f(&mut c.input);
        publish(&c);
        r
    };
    sync_backends();
    r
}

pub fn toggle() {
    let open = !lock().input.is_open();
    set_open(open);
}

/// Record that `c.input` changed its open state (a key did it) and tell Kettle.
pub fn publish(c: &Core) {
    let open = c.input.is_open();
    if OPEN.swap(open, Ordering::AcqRel) != open {
        ipc::send(&sr_overlay_proto::FromHook::Visible { open });
    }
}

/// Make SDL and Xlib match the open state. Called first thing by every input hook and by the renderers, in the
/// game's own thread, because the grabs must be changed from there.
pub fn sync_backends() {
    let open = is_open();
    if APPLIED.swap(open, Ordering::AcqRel) != open {
        let pos = lock().input.pos();
        crate::sdl::on_open(open, pos);
        crate::xlib::on_open(open, pos);
    }
}

pub fn set_data(d: OverlayData) {
    let mut c = lock();
    c.input.disabled = !d.enabled;
    c.data = d;
    let off = c.input.disabled && c.input.is_open();
    if off {
        c.input.set_open(false);
        publish(&c);
    }
}

impl Core {
    /// Note one present call (frame timing for the HUD).
    pub fn tick(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_present {
            self.worst = self.worst.max(now - last);
        }
        self.last_present = Some(now);
        self.presents.push_back(now);
        while self.presents.front().is_some_and(|t| now - *t > Duration::from_secs(1)) {
            self.presents.pop_front();
        }
        if self.worst_since.elapsed() >= Duration::from_secs(1) {
            self.worst_last = self.worst;
            self.worst = Duration::ZERO;
            self.worst_since = now;
        }
    }

    pub fn hud(&self) -> OverlayPerf {
        let n = self.presents.len();
        let (fps, ms) = match (self.presents.front(), self.presents.back()) {
            (Some(a), Some(b)) if n > 1 && b > a => {
                let span = (*b - *a).as_secs_f32();
                ((n - 1) as f32 / span, span * 1000.0 / (n - 1) as f32)
            }
            _ => (0.0, 0.0),
        };
        OverlayPerf { fps, frame_ms: ms }
    }

    /// What the overlay's look draws from: Kettle's snapshot, the clock and the measured frame rate.
    fn ui_data(&self) -> UiData {
        let d = &self.data;
        let (clock, date) = local_time();
        let _ = self.worst_last;
        UiData {
            app_id: d.app,
            game_name: d.game.clone(),
            session_minutes: (d.session_secs / 60) as u32,
            total_minutes: d.total_minutes,
            clock,
            date,
            account_name: d.me.clone(),
            friends: d
                .friends
                .iter()
                .map(|f| OverlayFriend {
                    steam_id: f.id,
                    name: f.name.clone(),
                    status: f.status.clone(),
                    in_game: f.in_game,
                    online: f.online,
                    avatar: None,
                })
                .collect(),
            achievements: (d.achievements_total > 0).then(|| OverlayAchievements {
                unlocked: d.achievements_unlocked,
                total: d.achievements_total,
                recent: d.achievements_recent.iter().map(|a| (a.name.clone(), a.description.clone(), a.time)).collect(),
            }),
            perf: Some(self.hud()),
        }
    }

    /// Run one egui frame for a surface of `width` x `height` pixels. `None` when there is nothing to draw
    /// (closed and no HUD).
    pub fn frame(&mut self, width: u32, height: u32) -> Option<Frame> {
        let open = self.input.is_open();
        let show_hud = self.data.hud || self.state.perf_hud;
        if !open && !show_hud {
            self.input.take_events();
            return None;
        }
        if width == 0 || height == 0 {
            return None;
        }
        let mut events = self.input.take_events();
        if !open {
            events.clear();
        }
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width as f32, height as f32))),
            time: Some(self.start.elapsed().as_secs_f64()),
            modifiers: self.input.modifiers(),
            events,
            focused: true,
            ..Default::default()
        };
        let mut actions: Vec<UiAction> = Vec::new();
        let hud = self.hud();
        let ui_data = if open { Some(self.ui_data()) } else { None };
        let state = &mut self.state;
        let out = self.ctx.run(raw, |ctx| match &ui_data {
            Some(d) => actions = sr_overlay_ui::show(ctx, d, state),
            None => sr_overlay_ui::show_hud(ctx, &hud),
        });
        let mut close = false;
        for a in actions {
            match a {
                UiAction::Close => close = true,
                UiAction::OpenUrl(url) => ipc::action(&OverlayAction::OpenUrl { url }),
                UiAction::OpenChat(id) => ipc::action(&OverlayAction::OpenChat { id }),
                UiAction::TakeScreenshot => ipc::action(&OverlayAction::Screenshot),
                UiAction::TogglePerfHud(on) => ipc::action(&OverlayAction::PerfHud { on }),
            }
        }
        if close {
            self.input.set_open(false);
            publish(self);
        }
        let ppp = out.pixels_per_point;
        let primitives = self.ctx.tessellate(out.shapes, ppp);
        // Some events arrive between frames: keep the context repainting while open.
        let _ = out.platform_output;
        Some(Frame { primitives, textures: out.textures_delta, pixels_per_point: ppp })
    }
}

/// The local time as "21:34" and "Thu, June 8, 2023".
fn local_time() -> (String, String) {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return (String::new(), String::new());
        }
        let fmt = |f: &std::ffi::CStr| {
            let mut buf = [0u8; 64];
            let n = libc::strftime(buf.as_mut_ptr().cast(), buf.len(), f.as_ptr(), &tm);
            String::from_utf8_lossy(&buf[..n]).into_owned()
        };
        (fmt(c"%H:%M"), fmt(c"%a, %B %-d, %Y"))
    }
}

/// Which surface (GL context or Vulkan swapchain) draws the overlay: one at a time, because egui's texture
/// updates go to one renderer only.
static OWNER: AtomicUsize = AtomicUsize::new(0);
static OWNER_SEEN_MS: AtomicU64 = AtomicU64::new(0);

#[derive(PartialEq, Eq, Debug)]
pub enum Claim {
    /// This surface draws, and already did.
    Owner,
    /// This surface draws from now on: the renderer must (re)upload everything (the egui state was reset).
    New,
    /// Another surface draws.
    Other,
}

fn now_ms() -> u64 {
    static T0: OnceLock<Instant> = OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

/// A surface is about to present. `id` is its handle (context or swapchain).
pub fn claim(id: usize) -> Claim {
    let now = now_ms();
    let owner = OWNER.load(Ordering::Acquire);
    let seen = OWNER_SEEN_MS.load(Ordering::Acquire);
    if owner == id {
        OWNER_SEEN_MS.store(now, Ordering::Release);
        return Claim::Owner;
    }
    if owner == 0 || now.saturating_sub(seen) > 2000 {
        OWNER.store(id, Ordering::Release);
        OWNER_SEEN_MS.store(now, Ordering::Release);
        lock().ctx = Context::default();
        return Claim::New;
    }
    Claim::Other
}

/// The surface `id` is gone.
pub fn release(id: usize) {
    let _ = OWNER.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
}
