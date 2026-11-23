//! SDL2 and SDL3: take the game's events while the overlay is open, and let go of the mouse (relative mode,
//! grabs, hidden cursor) until it closes. The game's own calls are remembered while open and applied again on
//! close, so it comes back exactly as it was.
//!
//! SDL2 and SDL3 export the same names with different event layouts; which one is loaded is told by an SDL3-only
//! function. SDL3 is written from its public headers and has not been run (no SDL3 here).

use crate::hooks;
use crate::input::{KeyKind, Verdict};
use crate::real::Slot;
use crate::state;
use egui::{Key, Pos2};
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// A window was made through SDL: SDL drives this process's input, so the Xlib hooks stay out of the way.
static USED: AtomicBool = AtomicBool::new(false);

pub fn in_use() -> bool {
    USED.load(Ordering::Acquire)
}

fn sdl3() -> bool {
    static YES: AtomicBool = AtomicBool::new(false);
    if YES.load(Ordering::Relaxed) {
        return true;
    }
    let y = slots::SDL_SetWindowRelativeMouseMode.real().is_some();
    if y {
        YES.store(true, Ordering::Relaxed);
    }
    y
}

/// The game's view of the mouse, kept while the overlay has changed the real one.
#[derive(Clone)]
struct Game {
    /// SDL2: one global relative mode; SDL3: per window (key 0 is used for SDL2).
    relative: HashMap<usize, bool>,
    /// None until the game says (the default is visible).
    cursor_shown: bool,
    mouse_grab: HashMap<usize, bool>,
    kbd_grab: HashMap<usize, bool>,
    text_input: bool,
}

fn game() -> std::sync::MutexGuard<'static, Game> {
    store().lock().unwrap_or_else(|e| e.into_inner())
}

fn store() -> &'static Mutex<Game> {
    static S: std::sync::OnceLock<Mutex<Game>> = std::sync::OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(Game {
            relative: HashMap::new(),
            cursor_shown: true,
            mouse_grab: HashMap::new(),
            kbd_grab: HashMap::new(),
            text_input: true,
        })
    })
}

thread_local! {
    /// `on_open` is changing the real state: SDL's own calls into our hooks must not count as the game's.
    static APPLYING: Cell<bool> = const { Cell::new(false) };
    /// We are inside one of our own SDL hooks (SDL's functions call each other through the symbol table).
    static INSIDE: Cell<u32> = const { Cell::new(0) };
}

struct Guard;
impl Guard {
    fn enter() -> (Guard, bool) {
        let outer = INSIDE.with(|d| {
            let v = d.get();
            d.set(v + 1);
            v == 0
        });
        (Guard, outer)
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        INSIDE.with(|d| d.set(d.get() - 1));
    }
}

unsafe fn real<F: Copy>(slot: &Slot) -> Option<F> {
    let p = slot.real()?;
    Some(unsafe { std::mem::transmute_copy::<usize, F>(&p) })
}

// ---- event decoding ---------------------------------------------------------------------------------------

unsafe fn rd<T: Copy>(ev: *const u8, off: usize) -> T {
    unsafe { std::ptr::read_unaligned(ev.add(off).cast::<T>()) }
}

/// Key kind and egui key for an SDL scancode (the numbers are the same in SDL2 and SDL3).
fn key_of(sc: u32) -> (KeyKind, Option<Key>) {
    use Key::*;
    let k = match sc {
        4..=29 => [A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z][(sc - 4) as usize],
        30..=38 => [Num1, Num2, Num3, Num4, Num5, Num6, Num7, Num8, Num9][(sc - 30) as usize],
        39 => Num0,
        40 | 88 => Enter,
        41 => return (KeyKind::Escape, Some(Escape)),
        42 => Backspace,
        43 => return (KeyKind::Tab, Some(Tab)),
        44 => Space,
        73 => Insert,
        74 => Home,
        75 => PageUp,
        76 => Delete,
        77 => End,
        78 => PageDown,
        79 => ArrowRight,
        80 => ArrowLeft,
        81 => ArrowDown,
        82 => ArrowUp,
        224 | 228 => return (KeyKind::Ctrl, None),
        225 | 229 => return (KeyKind::Shift, None),
        226 | 230 => return (KeyKind::Alt, None),
        69 => return (KeyKind::F12, None),
        _ => return (KeyKind::Other, None),
    };
    (KeyKind::Other, Some(k))
}

const SDL_KEYDOWN: u32 = 0x300;
const SDL_KEYUP: u32 = 0x301;
const SDL_TEXTEDITING: u32 = 0x302;
const SDL_TEXTINPUT: u32 = 0x303;
const SDL_MOUSEMOTION: u32 = 0x400;
const SDL_MOUSEBUTTONDOWN: u32 = 0x401;
const SDL_MOUSEBUTTONUP: u32 = 0x402;
const SDL_MOUSEWHEEL: u32 = 0x403;

/// Run one event through the overlay. `Swallow` = remove it from the game's stream.
unsafe fn filter(ev: *const u8) -> Verdict {
    let three = sdl3();
    let ty: u32 = unsafe { rd(ev, 0) };
    match ty {
        SDL_KEYDOWN | SDL_KEYUP => {
            let (sc, down, repeat): (u32, bool, bool) = unsafe {
                if three {
                    (rd(ev, 24), rd::<u8>(ev, 36) != 0, rd::<u8>(ev, 37) != 0)
                } else {
                    (rd::<i32>(ev, 16) as u32, ty == SDL_KEYDOWN, rd::<u8>(ev, 13) != 0)
                }
            };
            let (kind, key) = key_of(sc);
            state::with_input(|i| i.key(sc, kind, key, down, repeat)).verdict
        }
        SDL_TEXTINPUT => {
            let text = unsafe {
                let p = if three { rd::<usize>(ev, 24) as *const c_char } else { ev.add(12).cast::<c_char>() };
                if p.is_null() { String::new() } else { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
            };
            state::with_input(|i| i.text(&text))
        }
        SDL_TEXTEDITING => {
            if state::is_open() {
                Verdict::Swallow
            } else {
                Verdict::Pass
            }
        }
        SDL_MOUSEMOTION => {
            let (x, y) = unsafe {
                if three {
                    (rd::<f32>(ev, 28), rd::<f32>(ev, 32))
                } else {
                    (rd::<i32>(ev, 20) as f32, rd::<i32>(ev, 24) as f32)
                }
            };
            state::with_input(|i| i.motion(Pos2::new(x, y)))
        }
        SDL_MOUSEBUTTONDOWN | SDL_MOUSEBUTTONUP => {
            let (b, x, y) = unsafe {
                if three {
                    (rd::<u8>(ev, 24), rd::<f32>(ev, 28), rd::<f32>(ev, 32))
                } else {
                    (rd::<u8>(ev, 16), rd::<i32>(ev, 20) as f32, rd::<i32>(ev, 24) as f32)
                }
            };
            state::with_input(|i| i.button(b, ty == SDL_MOUSEBUTTONDOWN, Pos2::new(x, y)))
        }
        SDL_MOUSEWHEEL => {
            let (dx, dy) = unsafe {
                if three {
                    (rd::<f32>(ev, 24), rd::<f32>(ev, 28))
                } else {
                    (rd::<i32>(ev, 16) as f32, rd::<i32>(ev, 20) as f32)
                }
            };
            state::with_input(|i| i.wheel(dx, dy))
        }
        _ => Verdict::Pass,
    }
}

/// Drop swallowed events out of an array the game asked SDL to fill; returns the new count.
unsafe fn filter_array(events: *mut u8, n: c_int) -> c_int {
    if events.is_null() || n <= 0 {
        return n;
    }
    let stride = event_size();
    let mut kept = 0usize;
    for i in 0..n as usize {
        let e = unsafe { events.add(i * stride) };
        if unsafe { filter(e) } == Verdict::Pass {
            if kept != i {
                unsafe { std::ptr::copy_nonoverlapping(e, events.add(kept * stride), stride) };
            }
            kept += 1;
        }
    }
    kept as c_int
}

/// `sizeof(SDL_Event)` in SDL2 (56) and SDL3 (128).
fn event_size() -> usize {
    if sdl3() { 128 } else { 56 }
}

// ---- grab release and restore ----------------------------------------------------------------------------

/// How many device pixels one point of the game's window is (hi-dpi), from SDL's own two window sizes: the
/// window's size in points (what its mouse coordinates use) and in pixels (what the game renders). Asked at most
/// twice a second, and only when SDL runs the game's window. 1.0 when SDL cannot tell.
pub fn update_scale() {
    use std::sync::atomic::AtomicU64;
    static LAST: AtomicU64 = AtomicU64::new(0);
    if !in_use() {
        return;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let last = LAST.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 500 || LAST.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err()
    {
        return;
    }
    type Focus = unsafe extern "C" fn() -> *mut c_void;
    type Size = unsafe extern "C" fn(*mut c_void, *mut c_int, *mut c_int);
    let get = |n: &std::ffi::CStr| crate::real::sym_in(n, &[c"libSDL2-2.0.so.0", c"libSDL3.so.0"]);
    let (Some(size), Some(pixels)) = (get(c"SDL_GetWindowSize"), get(c"SDL_GetWindowSizeInPixels")) else { return };
    let focus = get(c"SDL_GetMouseFocus").or_else(|| get(c"SDL_GetKeyboardFocus"));
    let Some(focus) = focus else { return };
    unsafe {
        let win = std::mem::transmute::<usize, Focus>(focus)();
        if win.is_null() {
            return;
        }
        let (mut w, mut h, mut pw, mut ph) = (0, 0, 0, 0);
        std::mem::transmute::<usize, Size>(size)(win, &mut w, &mut h);
        std::mem::transmute::<usize, Size>(pixels)(win, &mut pw, &mut ph);
        if w > 0 && pw > 0 {
            state::set_scale(pw as f32 / w as f32);
        }
    }
}

/// The overlay opens (`open`) or closes. Runs in the game's thread.
pub fn on_open(open: bool, pos: Pos2) {
    if !in_use() {
        return;
    }
    let three = sdl3();
    let g = game().clone();
    log_real(if open { "before-open" } else { "before-close" }, &g, three);
    APPLYING.with(|a| a.set(true));
    let mut new_pos = None;
    unsafe {
        if open {
            if three {
                if let Some(f) =
                    real::<unsafe extern "C" fn(*mut c_void, bool) -> bool>(&slots::SDL_SetWindowRelativeMouseMode)
                {
                    for (win, on) in &g.relative {
                        if *on {
                            f(*win as *mut c_void, false);
                        }
                    }
                }
                if let Some(f) = real::<unsafe extern "C" fn() -> bool>(&slots::SDL_ShowCursor) {
                    f();
                }
                if let Some(f) = real::<unsafe extern "C" fn(*mut c_void, bool) -> bool>(&slots::SDL_SetWindowMouseGrab)
                {
                    for (win, on) in &g.mouse_grab {
                        if *on {
                            f(*win as *mut c_void, false);
                        }
                    }
                }
                if let Some(f) =
                    real::<unsafe extern "C" fn(*mut c_void, bool) -> bool>(&slots::SDL_SetWindowKeyboardGrab)
                {
                    for (win, on) in &g.kbd_grab {
                        if *on {
                            f(*win as *mut c_void, false);
                        }
                    }
                }
            } else {
                if g.relative.get(&0).copied().unwrap_or(false)
                    && let Some(f) = real::<unsafe extern "C" fn(c_int) -> c_int>(&slots::SDL_SetRelativeMouseMode)
                {
                    f(0);
                }
                if let Some(f) = real::<unsafe extern "C" fn(c_int) -> c_int>(&slots::SDL_ShowCursor) {
                    f(1);
                }
                if let Some(f) = real::<unsafe extern "C" fn(*mut c_void, c_int)>(&slots::SDL_SetWindowGrab) {
                    for (win, on) in g.mouse_grab.iter().chain(g.kbd_grab.iter()) {
                        if *on {
                            f(*win as *mut c_void, 0);
                        }
                    }
                }
                if !g.text_input
                    && let Some(f) = real::<unsafe extern "C" fn()>(&slots::SDL_StartTextInput)
                {
                    f();
                }
                if let Some(f) = real::<unsafe extern "C" fn(*mut c_int, *mut c_int) -> u32>(&slots::SDL_GetMouseState)
                {
                    let (mut x, mut y) = (0, 0);
                    f(&mut x, &mut y);
                    new_pos = Some(Pos2::new(x as f32, y as f32));
                }
            }
        } else if three {
            if let Some(f) = real::<unsafe extern "C" fn(*mut c_void, bool) -> bool>(&slots::SDL_SetWindowMouseGrab) {
                for (win, on) in &g.mouse_grab {
                    f(*win as *mut c_void, *on);
                }
            }
            if let Some(f) = real::<unsafe extern "C" fn(*mut c_void, bool) -> bool>(&slots::SDL_SetWindowKeyboardGrab)
            {
                for (win, on) in &g.kbd_grab {
                    f(*win as *mut c_void, *on);
                }
            }
            if !g.cursor_shown
                && let Some(f) = real::<unsafe extern "C" fn() -> bool>(&slots::SDL_HideCursor)
            {
                f();
            }
            if let Some(f) =
                real::<unsafe extern "C" fn(*mut c_void, bool) -> bool>(&slots::SDL_SetWindowRelativeMouseMode)
            {
                for (win, on) in &g.relative {
                    if *on {
                        f(*win as *mut c_void, true);
                    }
                }
            }
        } else {
            if let Some(f) = real::<unsafe extern "C" fn(*mut c_void, c_int)>(&slots::SDL_SetWindowGrab) {
                for win in g.mouse_grab.keys().chain(g.kbd_grab.keys()).collect::<std::collections::BTreeSet<_>>() {
                    let on = g.mouse_grab.get(win).copied().unwrap_or(false)
                        || g.kbd_grab.get(win).copied().unwrap_or(false);
                    f(*win as *mut c_void, on as c_int);
                }
            }
            if let Some(f) = real::<unsafe extern "C" fn(c_int) -> c_int>(&slots::SDL_ShowCursor) {
                f(g.cursor_shown as c_int);
            }
            if !g.text_input
                && let Some(f) = real::<unsafe extern "C" fn()>(&slots::SDL_StopTextInput)
            {
                f();
            }
            if g.relative.get(&0).copied().unwrap_or(false)
                && let Some(f) = real::<unsafe extern "C" fn(c_int) -> c_int>(&slots::SDL_SetRelativeMouseMode)
            {
                f(1);
            }
        }
    }
    APPLYING.with(|a| a.set(false));
    if let Some(p) = new_pos {
        state::lock().input.set_pos(p);
    }
    log_real(if open { "open" } else { "close" }, &g, three);
    let _ = pos;
}

fn applying() -> bool {
    APPLYING.with(Cell::get)
}

/// For the tests: what the real SDL state is after switching (read with SDL's own functions).
fn log_real(label: &str, g: &Game, three: bool) {
    if std::env::var_os("KETTLE_OVERLAY_LOG").is_none() || three {
        return;
    }
    unsafe {
        let rel = real::<unsafe extern "C" fn() -> c_int>(&slots::SDL_GetRelativeMouseMode).map(|f| f());
        let cursor = real::<unsafe extern "C" fn(c_int) -> c_int>(&slots::SDL_ShowCursor).map(|f| f(-1));
        let grab = real::<unsafe extern "C" fn(*mut c_void) -> c_int>(&slots::SDL_GetWindowGrab)
            .map(|f| g.mouse_grab.keys().map(|w| f(*w as *mut c_void)).max().unwrap_or(0));
        crate::ipc::log(&format!("sdl {label} real rel={rel:?} cursor={cursor:?} grab={grab:?}"));
    }
}

hooks! {
    fn SDL_PollEvent(real; ev: *mut u8) -> c_int {
        let (_g, outer) = Guard::enter();
        loop {
            let r = unsafe { real(ev) };
            if r == 0 || !outer || ev.is_null() {
                return r;
            }
            if unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn SDL_WaitEvent(real; ev: *mut u8) -> c_int {
        let (_g, outer) = Guard::enter();
        loop {
            let r = unsafe { real(ev) };
            if r == 0 || !outer || ev.is_null() {
                return r;
            }
            if unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
        }
    }
    fn SDL_WaitEventTimeout(real; ev: *mut u8, timeout: c_int) -> c_int {
        let (_g, outer) = Guard::enter();
        let end = std::time::Instant::now() + std::time::Duration::from_millis(timeout.max(0) as u64);
        let mut left = timeout;
        loop {
            let r = unsafe { real(ev, left) };
            if r == 0 || !outer || ev.is_null() {
                return r;
            }
            if unsafe { filter(ev) } == Verdict::Pass {
                return r;
            }
            if timeout >= 0 {
                let now = std::time::Instant::now();
                if now >= end {
                    return 0;
                }
                left = (end - now).as_millis() as c_int;
            }
        }
    }
    fn SDL_PeepEvents(real; events: *mut u8, n: c_int, action: c_int, min: u32, max: u32) -> c_int {
        let (_g, outer) = Guard::enter();
        let r = unsafe { real(events, n, action, min, max) };
        // Only events taken out of the queue are filtered; a peek leaves them for the next poll.
        if outer && action == 2 && r > 0 {
            return unsafe { filter_array(events, r) };
        }
        r
    }
    fn SDL_CreateWindow(real; a: usize, b: usize, c: usize, d: usize, e: usize, f: usize) -> *mut c_void {
        let w = unsafe { real(a, b, c, d, e, f) };
        if !w.is_null() {
            USED.store(true, Ordering::Release);
        }
        w
    }
    fn SDL_CreateWindowWithProperties(real; props: u32) -> *mut c_void {
        let w = unsafe { real(props) };
        if !w.is_null() {
            USED.store(true, Ordering::Release);
        }
        w
    }
    fn SDL_SetRelativeMouseMode(real; on: c_int) -> c_int {
        if applying() {
            return unsafe { real(on) };
        }
        if state::is_open() {
            game().relative.insert(0, on != 0);
            return 0;
        }
        let r = unsafe { real(on) };
        if r == 0 {
            game().relative.insert(0, on != 0);
        }
        r
    }
    fn SDL_GetRelativeMouseMode(real; ) -> c_int {
        if state::is_open() {
            return game().relative.get(&0).copied().unwrap_or(false) as c_int;
        }
        unsafe { real() }
    }
    fn SDL_SetWindowRelativeMouseMode(real; win: *mut c_void, on: bool) -> bool {
        if applying() {
            return unsafe { real(win, on) };
        }
        if state::is_open() {
            game().relative.insert(win as usize, on);
            return true;
        }
        let r = unsafe { real(win, on) };
        if r {
            game().relative.insert(win as usize, on);
        }
        r
    }
    fn SDL_GetWindowRelativeMouseMode(real; win: *mut c_void) -> bool {
        if state::is_open() {
            return game().relative.get(&(win as usize)).copied().unwrap_or(false);
        }
        unsafe { real(win) }
    }
    fn SDL_ShowCursor(real; toggle: c_int) -> c_int {
        if applying() {
            return unsafe { real(toggle) };
        }
        if sdl3() {
            // SDL3's SDL_ShowCursor takes nothing and returns a bool.
            if !state::is_open() {
                game().cursor_shown = true;
            }
            return unsafe { real(toggle) };
        }
        let mut g = game();
        let was = g.cursor_shown as c_int;
        if toggle >= 0 {
            g.cursor_shown = toggle != 0;
        }
        if state::is_open() {
            return was;
        }
        drop(g);
        unsafe { real(toggle) }
    }
    fn SDL_HideCursor(real; ) -> bool {
        if applying() {
            return unsafe { real() };
        }
        if state::is_open() {
            game().cursor_shown = false;
            return true;
        }
        game().cursor_shown = false;
        unsafe { real() }
    }
    fn SDL_CursorVisible(real; ) -> bool {
        if state::is_open() {
            return game().cursor_shown;
        }
        unsafe { real() }
    }
    fn SDL_SetWindowGrab(real; win: *mut c_void, on: c_int) -> () {
        if applying() {
            return unsafe { real(win, on) };
        }
        {
            let mut g = game();
            g.mouse_grab.insert(win as usize, on != 0);
            g.kbd_grab.insert(win as usize, on != 0);
        }
        if !state::is_open() {
            unsafe { real(win, on) }
        }
    }
    fn SDL_SetWindowMouseGrab(real; win: *mut c_void, on: c_int) -> c_int {
        if applying() {
            return unsafe { real(win, on) };
        }
        game().mouse_grab.insert(win as usize, on != 0);
        if state::is_open() {
            return if sdl3() { 1 } else { 0 };
        }
        unsafe { real(win, on) }
    }
    fn SDL_SetWindowKeyboardGrab(real; win: *mut c_void, on: c_int) -> c_int {
        if applying() {
            return unsafe { real(win, on) };
        }
        game().kbd_grab.insert(win as usize, on != 0);
        if state::is_open() {
            return if sdl3() { 1 } else { 0 };
        }
        unsafe { real(win, on) }
    }
    fn SDL_GetWindowGrab(real; win: *mut c_void) -> c_int {
        if state::is_open() {
            let g = game();
            return (g.mouse_grab.get(&(win as usize)).copied().unwrap_or(false)
                || g.kbd_grab.get(&(win as usize)).copied().unwrap_or(false)) as c_int;
        }
        unsafe { real(win) }
    }
    fn SDL_StartTextInput(real; win: *mut c_void) -> () {
        if applying() {
            return unsafe { real(win) };
        }
        if !sdl3() {
            game().text_input = true;
        }
        unsafe { real(win) }
    }
    fn SDL_StopTextInput(real; win: *mut c_void) -> () {
        if applying() {
            return unsafe { real(win) };
        }
        if !sdl3() {
            game().text_input = false;
        }
        if state::is_open() && !sdl3() {
            return;
        }
        unsafe { real(win) }
    }
    fn SDL_GetKeyboardState(real; numkeys: *mut c_int) -> *const u8 {
        if state::is_open() {
            static NONE: [u8; 512] = [0; 512];
            if !numkeys.is_null() {
                unsafe { *numkeys = 512 };
            }
            return NONE.as_ptr();
        }
        unsafe { real(numkeys) }
    }
    fn SDL_GetMouseState(real; x: *mut c_int, y: *mut c_int) -> u32 {
        let b = unsafe { real(x, y) };
        if state::is_open() { 0 } else { b }
    }
    fn SDL_GetGlobalMouseState(real; x: *mut c_int, y: *mut c_int) -> u32 {
        let b = unsafe { real(x, y) };
        if state::is_open() { 0 } else { b }
    }
    fn SDL_GetRelativeMouseState(real; x: *mut c_int, y: *mut c_int) -> u32 {
        let b = unsafe { real(x, y) };
        if state::is_open() {
            // Zero is the same four bytes for SDL3's floats.
            unsafe {
                if !x.is_null() {
                    *x = 0;
                }
                if !y.is_null() {
                    *y = 0;
                }
            }
            return 0;
        }
        b
    }
}
