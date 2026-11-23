//! Xlib: the input path of Wine and Proton games (winex11) and of GLFW or hand-written X11 programs. Takes the
//! game's events while the overlay is open, lets go of pointer and keyboard grabs, shows the cursor, and puts
//! everything back on close.
//!
//! Programs that create their windows through SDL are handled by the SDL hooks instead (SDL calls Xlib itself,
//! and swallowing events there would break SDL's own bookkeeping); see `sdl::in_use`.

use crate::hooks;
use crate::input::{KeyKind, Verdict};
use crate::real;
use crate::state;
use egui::{Key, Pos2};
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_uint, c_ulong, c_void};
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Display = *mut c_void;
type Window = c_ulong;

const KEY_PRESS: c_int = 2;
const KEY_RELEASE: c_int = 3;
const BUTTON_PRESS: c_int = 4;
const BUTTON_RELEASE: c_int = 5;
const MOTION_NOTIFY: c_int = 6;
const NO_EXPOSE: c_int = 14;
const GENERIC_EVENT: c_int = 35;
/// The size of an XEvent on 64-bit Linux.
const EVENT_BYTES: usize = 192;

fn active() -> bool {
    !crate::sdl::in_use()
}

unsafe fn rd<T: Copy>(ev: *const u8, off: usize) -> T {
    unsafe { std::ptr::read_unaligned(ev.add(off).cast::<T>()) }
}

fn key_of(sym: u64) -> (KeyKind, Option<Key>) {
    use Key::*;
    let k = match sym {
        0xff09 | 0xfe20 => return (KeyKind::Tab, Some(Tab)),
        0xff1b => return (KeyKind::Escape, Some(Escape)),
        0xffe1 | 0xffe2 => return (KeyKind::Shift, None),
        0xffe3 | 0xffe4 => return (KeyKind::Ctrl, None),
        0xffe9 | 0xffea | 0xffe7 | 0xffe8 | 0xfe03 => return (KeyKind::Alt, None),
        0xff0d | 0xff8d => Enter,
        0xff08 => Backspace,
        0xffff => Delete,
        0xff50 => Home,
        0xff57 => End,
        0xff55 => PageUp,
        0xff56 => PageDown,
        0xff63 => Insert,
        0xff51 => ArrowLeft,
        0xff52 => ArrowUp,
        0xff53 => ArrowRight,
        0xff54 => ArrowDown,
        0x20 => Space,
        0x61..=0x7a => {
            [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z][(sym - 0x61) as usize]
        }
        0x41..=0x5a => {
            [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z][(sym - 0x41) as usize]
        }
        0x30..=0x39 => [Num0, Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9][(sym - 0x30) as usize],
        _ => return (KeyKind::Other, None),
    };
    (KeyKind::Other, Some(k))
}

/// What the game believes about grabs and cursors, kept while the overlay has changed the real ones.
#[derive(Default)]
struct State {
    dpy: usize,
    win: Window,
    /// `XGrabPointer` arguments of the grab the game holds.
    ptr_grab: Option<[c_ulong; 8]>,
    /// `XGrabKeyboard` arguments.
    kbd_grab: Option<[c_ulong; 4]>,
    cursors: HashMap<Window, c_ulong>,
    hidden: HashMap<Window, u32>,
    last_toggle: Option<Instant>,
}

fn state() -> std::sync::MutexGuard<'static, State> {
    static S: std::sync::OnceLock<Mutex<State>> = std::sync::OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default())).lock().unwrap_or_else(|e| e.into_inner())
}

unsafe fn f<F: Copy>(slot: &real::Slot) -> Option<F> {
    let p = slot.real()?;
    Some(unsafe { std::mem::transmute_copy::<usize, F>(&p) })
}

unsafe fn lib<F: Copy>(name: &std::ffi::CStr, libs: &[&std::ffi::CStr]) -> Option<F> {
    let p = real::sym_in(name, libs)?;
    Some(unsafe { std::mem::transmute_copy::<usize, F>(&p) })
}

/// Run one event through the overlay.
unsafe fn filter(ev: *const u8) -> Verdict {
    let ty: c_int = unsafe { rd(ev, 0) };
    {
        let mut s = state();
        match ty {
            KEY_PRESS | KEY_RELEASE | BUTTON_PRESS | BUTTON_RELEASE | MOTION_NOTIFY => {
                s.dpy = unsafe { rd::<usize>(ev, 24) };
                s.win = unsafe { rd::<Window>(ev, 32) };
            }
            _ => {}
        }
    }
    match ty {
        KEY_PRESS | KEY_RELEASE => {
            let keycode: u32 = unsafe { rd(ev, 84) };
            let sym = unsafe {
                match lib::<unsafe extern "C" fn(*const u8, c_int) -> c_ulong>(c"XLookupKeysym", &[c"libX11.so.6"]) {
                    Some(l) => l(ev, 0),
                    None => 0,
                }
            };
            let (kind, key) = key_of(sym);
            let down = ty == KEY_PRESS;
            // Held Shift+Tab arrives as release/press pairs: a second toggle this soon is the key repeating.
            if down && kind == KeyKind::Tab {
                let mut s = state();
                if s.last_toggle.is_some_and(|t| t.elapsed() < Duration::from_millis(250)) {
                    s.last_toggle = Some(Instant::now());
                    return Verdict::Swallow;
                }
            }
            let step = state::with_input(|i| i.key(keycode, kind, key, down, false));
            if step.toggled.is_some() {
                state().last_toggle = Some(Instant::now());
            }
            if down && step.verdict == Verdict::Swallow && state::is_open() && step.toggled.is_none() {
                // Text for egui (Latin-1 as the server gives it).
                let mut buf = [0u8; 16];
                let n = unsafe {
                    match lib::<unsafe extern "C" fn(*const u8, *mut u8, c_int, *mut c_ulong, *mut c_void) -> c_int>(
                        c"XLookupString",
                        &[c"libX11.so.6"],
                    ) {
                        Some(l) => l(ev, buf.as_mut_ptr(), 16, std::ptr::null_mut(), std::ptr::null_mut()),
                        None => 0,
                    }
                };
                if n > 0 {
                    let t: String = buf[..n as usize].iter().map(|b| *b as char).collect();
                    state::with_input(|i| i.text(&t));
                }
            }
            step.verdict
        }
        BUTTON_PRESS | BUTTON_RELEASE => {
            let (x, y): (c_int, c_int) = unsafe { (rd(ev, 64), rd(ev, 68)) };
            let b: u32 = unsafe { rd(ev, 84) };
            let pos = Pos2::new(x as f32, y as f32);
            let down = ty == BUTTON_PRESS;
            state::with_input(|i| match b {
                4..=7 => {
                    if !i.is_open() {
                        Verdict::Pass
                    } else {
                        if down {
                            let (dx, dy) = match b {
                                4 => (0.0, 1.0),
                                5 => (0.0, -1.0),
                                6 => (1.0, 0.0),
                                _ => (-1.0, 0.0),
                            };
                            i.wheel(dx, dy);
                        }
                        Verdict::Swallow
                    }
                }
                8 | 9 => i.button(b as u8 - 4, down, pos),
                _ => i.button(b as u8, down, pos),
            })
        }
        MOTION_NOTIFY => {
            let (x, y): (c_int, c_int) = unsafe { (rd(ev, 64), rd(ev, 68)) };
            state::with_input(|i| i.motion(Pos2::new(x as f32, y as f32)))
        }
        GENERIC_EVENT => {
            // XInput2 key, button, motion and raw versions of them: the game must not see them while open.
            let evtype: c_int = unsafe { rd(ev, 36) };
            if state::is_open() && matches!(evtype, 2..=6 | 13..=17 | 18..=24) {
                Verdict::Swallow
            } else {
                Verdict::Pass
            }
        }
        _ => Verdict::Pass,
    }
}

/// A harmless event for a caller that was promised one (its queue had only swallowed events).
unsafe fn fill_noop(dpy: Display, ev: *mut u8) {
    unsafe {
        std::ptr::write_bytes(ev, 0, EVENT_BYTES);
        std::ptr::write_unaligned(ev.cast::<c_int>(), NO_EXPOSE);
        std::ptr::write_unaligned(ev.add(24).cast::<usize>(), dpy as usize);
    }
}

unsafe fn pending(dpy: Display) -> c_int {
    match unsafe { lib::<unsafe extern "C" fn(Display) -> c_int>(c"XPending", &[c"libX11.so.6"]) } {
        Some(p) => unsafe { p(dpy) },
        None => 0,
    }
}

// ---- grab release and restore ----------------------------------------------------------------------------

/// The overlay opens or closes. Runs in the game's thread.
pub fn on_open(open: bool, _pos: Pos2) {
    if !active() {
        return;
    }
    let s = state();
    let dpy = s.dpy as Display;
    if dpy.is_null() {
        return;
    }
    unsafe {
        let flush = lib::<unsafe extern "C" fn(Display) -> c_int>(c"XFlush", &[c"libX11.so.6"]);
        let ungrab_p = f::<unsafe extern "C" fn(Display, c_ulong) -> c_int>(&slots::XUngrabPointer);
        let ungrab_k = f::<unsafe extern "C" fn(Display, c_ulong) -> c_int>(&slots::XUngrabKeyboard);
        let define = f::<unsafe extern "C" fn(Display, Window, c_ulong) -> c_int>(&slots::XDefineCursor);
        let undefine = f::<unsafe extern "C" fn(Display, Window) -> c_int>(&slots::XUndefineCursor);
        let hide = f::<unsafe extern "C" fn(Display, Window)>(&slots::XFixesHideCursor);
        let show = f::<unsafe extern "C" fn(Display, Window)>(&slots::XFixesShowCursor);
        if open {
            if s.ptr_grab.is_some()
                && let Some(u) = ungrab_p
            {
                u(dpy, 0);
            }
            if s.kbd_grab.is_some()
                && let Some(u) = ungrab_k
            {
                u(dpy, 0);
            }
            if let Some(u) = undefine {
                for (w, c) in &s.cursors {
                    if *c != 0 {
                        u(dpy, *w);
                    }
                }
            }
            if let Some(sh) = show {
                for (w, n) in &s.hidden {
                    for _ in 0..*n {
                        sh(dpy, *w);
                    }
                }
            }
            // The pointer position for egui.
            if let Some(q) = lib::<
                unsafe extern "C" fn(
                    Display,
                    Window,
                    *mut Window,
                    *mut Window,
                    *mut c_int,
                    *mut c_int,
                    *mut c_int,
                    *mut c_int,
                    *mut c_uint,
                ) -> c_int,
            >(c"XQueryPointer", &[c"libX11.so.6"])
            {
                let (mut r, mut c, mut rx, mut ry, mut wx, mut wy, mut m) = (0, 0, 0, 0, 0, 0, 0);
                if q(dpy, s.win, &mut r, &mut c, &mut rx, &mut ry, &mut wx, &mut wy, &mut m) != 0 {
                    drop(s);
                    state::lock().input.set_pos(Pos2::new(wx as f32, wy as f32));
                    if let Some(fl) = flush {
                        fl(dpy);
                    }
                    return;
                }
            }
        } else {
            if let Some(g) = s.ptr_grab
                && let Some(gp) = f::<
                    unsafe extern "C" fn(
                        Display,
                        Window,
                        c_int,
                        c_uint,
                        c_int,
                        c_int,
                        Window,
                        c_ulong,
                        c_ulong,
                    ) -> c_int,
                >(&slots::XGrabPointer)
            {
                gp(dpy, g[0], g[1] as c_int, g[2] as c_uint, g[3] as c_int, g[4] as c_int, g[5], g[6], 0);
            }
            if let Some(g) = s.kbd_grab
                && let Some(gk) = f::<unsafe extern "C" fn(Display, Window, c_int, c_int, c_int, c_ulong) -> c_int>(
                    &slots::XGrabKeyboard,
                )
            {
                gk(dpy, g[0], g[1] as c_int, g[2] as c_int, g[3] as c_int, 0);
            }
            if let Some(d) = define {
                for (w, c) in &s.cursors {
                    if *c != 0 {
                        d(dpy, *w, *c);
                    }
                }
            }
            if let Some(h) = hide {
                for (w, n) in &s.hidden {
                    for _ in 0..*n {
                        h(dpy, *w);
                    }
                }
            }
        }
        if let Some(fl) = flush {
            fl(dpy);
        }
    }
}

hooks! {
    fn XNextEvent(real; dpy: Display, ev: *mut u8) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, ev) };
            if !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
            if unsafe { pending(dpy) } == 0 {
                unsafe { fill_noop(dpy, ev) };
                return r;
            }
        }
    }
    fn XIfEvent(real; dpy: Display, ev: *mut u8, pred: *mut c_void, arg: *mut c_void) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, ev, pred, arg) };
            if !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XCheckIfEvent(real; dpy: Display, ev: *mut u8, pred: *mut c_void, arg: *mut c_void) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, ev, pred, arg) };
            if r == 0 || !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XWindowEvent(real; dpy: Display, w: Window, mask: c_ulong, ev: *mut u8) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, w, mask, ev) };
            if !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XCheckWindowEvent(real; dpy: Display, w: Window, mask: c_ulong, ev: *mut u8) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, w, mask, ev) };
            if r == 0 || !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XMaskEvent(real; dpy: Display, mask: c_ulong, ev: *mut u8) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, mask, ev) };
            if !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XCheckMaskEvent(real; dpy: Display, mask: c_ulong, ev: *mut u8) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, mask, ev) };
            if r == 0 || !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XCheckTypedEvent(real; dpy: Display, ty: c_int, ev: *mut u8) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, ty, ev) };
            if r == 0 || !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XCheckTypedWindowEvent(real; dpy: Display, w: Window, ty: c_int, ev: *mut u8) -> c_int {
        state::sync_backends();
        loop {
            let r = unsafe { real(dpy, w, ty, ev) };
            if r == 0 || !active() || unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn XGrabPointer(real; dpy: Display, w: Window, owner: c_int, mask: c_uint, pm: c_int, km: c_int, confine: Window, cursor: c_ulong, time: c_ulong) -> c_int {
        if !active() {
            return unsafe { real(dpy, w, owner, mask, pm, km, confine, cursor, time) };
        }
        {
            let mut s = state();
            s.dpy = dpy as usize;
        }
        if state::is_open() {
            state().ptr_grab = Some([w, owner as c_ulong, mask as c_ulong, pm as c_ulong, km as c_ulong, confine, cursor, 0]);
            return 0;
        }
        let r = unsafe { real(dpy, w, owner, mask, pm, km, confine, cursor, time) };
        if r == 0 {
            state().ptr_grab = Some([w, owner as c_ulong, mask as c_ulong, pm as c_ulong, km as c_ulong, confine, cursor, 0]);
        }
        r
    }
    fn XUngrabPointer(real; dpy: Display, time: c_ulong) -> c_int {
        if active() {
            state().ptr_grab = None;
            if state::is_open() {
                return 1;
            }
        }
        unsafe { real(dpy, time) }
    }
    fn XGrabKeyboard(real; dpy: Display, w: Window, owner: c_int, pm: c_int, km: c_int, time: c_ulong) -> c_int {
        if !active() {
            return unsafe { real(dpy, w, owner, pm, km, time) };
        }
        state().dpy = dpy as usize;
        if state::is_open() {
            state().kbd_grab = Some([w, owner as c_ulong, pm as c_ulong, km as c_ulong]);
            return 0;
        }
        let r = unsafe { real(dpy, w, owner, pm, km, time) };
        if r == 0 {
            state().kbd_grab = Some([w, owner as c_ulong, pm as c_ulong, km as c_ulong]);
        }
        r
    }
    fn XUngrabKeyboard(real; dpy: Display, time: c_ulong) -> c_int {
        if active() {
            state().kbd_grab = None;
            if state::is_open() {
                return 1;
            }
        }
        unsafe { real(dpy, time) }
    }
    fn XWarpPointer(real; dpy: Display, sw: Window, dw: Window, sx: c_int, sy: c_int, sw2: c_uint, sh: c_uint, dx: c_int, dy: c_int) -> c_int {
        if active() && state::is_open() {
            return 1;
        }
        unsafe { real(dpy, sw, dw, sx, sy, sw2, sh, dx, dy) }
    }
    fn XDefineCursor(real; dpy: Display, w: Window, cursor: c_ulong) -> c_int {
        if active() {
            state().cursors.insert(w, cursor);
            if state::is_open() {
                return 1;
            }
        }
        unsafe { real(dpy, w, cursor) }
    }
    fn XUndefineCursor(real; dpy: Display, w: Window) -> c_int {
        if active() {
            state().cursors.remove(&w);
        }
        unsafe { real(dpy, w) }
    }
    fn XFixesHideCursor(real; dpy: Display, w: Window) -> () {
        if active() {
            *state().hidden.entry(w).or_insert(0) += 1;
            if state::is_open() {
                return;
            }
        }
        unsafe { real(dpy, w) }
    }
    fn XFixesShowCursor(real; dpy: Display, w: Window) -> () {
        if active() {
            let mut s = state();
            if let Some(n) = s.hidden.get_mut(&w) {
                *n = n.saturating_sub(1);
            }
            drop(s);
            if state::is_open() {
                return;
            }
        }
        unsafe { real(dpy, w) }
    }
}

#[allow(dead_code)]
fn _types(_: *const c_char) {}
