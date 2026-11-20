//! The global Shift+Tab shortcut for the in-game overlay.
//!
//! It exists only while a game we started runs: [`Hotkey::set_active`] registers it when the first game starts and
//! releases it when none runs, so other programs keep their Shift+Tab the rest of the time. One worker thread per
//! registration; it calls `on_toggle` for each key press (the app turns that into a message for the UI plus one
//! `request_repaint`, so nothing polls).
//!
//! Backends, chosen at run time: the XDG GlobalShortcuts portal (Wayland), an X11 key grab on the root window
//! (X11 and XWayland), `RegisterHotKey` (Windows). If one fails the next candidate is tried; if all fail we log
//! once and go on (the overlay is then reachable from the Games menu).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// `org.freedesktop.portal.GlobalShortcuts` (KDE, GNOME on Wayland).
    Portal,
    /// `XGrabKey` on the root window.
    X11,
    /// `RegisterHotKey` on a thread with its own message loop.
    Windows,
}

/// Backends to try, best first, from the environment (`WAYLAND_DISPLAY`, `XDG_SESSION_TYPE`, `DISPLAY`).
/// Pure, so it can be tested; the caller passes the variables in.
pub fn candidates(wayland_display: Option<&str>, session_type: Option<&str>, display: Option<&str>) -> Vec<Backend> {
    if cfg!(windows) {
        return vec![Backend::Windows];
    }
    let set = |v: Option<&str>| v.is_some_and(|s| !s.is_empty());
    let wayland = set(wayland_display) || session_type.is_some_and(|s| s.eq_ignore_ascii_case("wayland"));
    let mut out = Vec::new();
    if wayland {
        out.push(Backend::Portal);
    }
    // On Wayland this only sees keys while an XWayland window has focus, but it costs nothing to try.
    if set(display) {
        out.push(Backend::X11);
    }
    out
}

fn env_candidates() -> Vec<Backend> {
    let get = |k: &str| std::env::var(k).ok();
    candidates(get("WAYLAND_DISPLAY").as_deref(), get("XDG_SESSION_TYPE").as_deref(), get("DISPLAY").as_deref())
}

/// X11 modifier bits.
#[cfg(all(unix, not(target_os = "macos")))]
const SHIFT: u16 = 1;
#[cfg(all(unix, not(target_os = "macos")))]
const LOCK: u16 = 1 << 1;
#[cfg(all(unix, not(target_os = "macos")))]
/// NumLock is Mod2 on practically every setup.
const MOD2: u16 = 1 << 4;

#[cfg(all(unix, not(target_os = "macos")))]
/// `base` with every combination of CapsLock and NumLock, because a grab only matches the exact modifier state.
pub fn mask_variants(base: u16) -> [u16; 4] {
    [base, base | LOCK, base | MOD2, base | LOCK | MOD2]
}

#[cfg(all(unix, not(target_os = "macos")))]
/// The first keycode whose keysym list holds `sym`, from a `GetKeyboardMapping` reply.
pub fn keycode_for(min_keycode: u8, per_keycode: usize, keysyms: &[u32], sym: u32) -> Option<u8> {
    if per_keycode == 0 {
        return None;
    }
    let idx = keysyms.chunks(per_keycode).position(|c| c.contains(&sym))?;
    u8::try_from(idx + usize::from(min_keycode)).ok()
}

/// Stops a worker: sets a flag and wakes whatever the worker is blocked in.
#[derive(Default)]
struct Stop {
    flag: AtomicBool,
    wake: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl Stop {
    fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    fn set(&self) {
        self.flag.store(true, Ordering::SeqCst);
        let wake = self.wake.lock().ok().and_then(|mut w| w.take());
        if let Some(w) = wake {
            w();
        }
    }

    /// Register how to wake the worker; runs at once when stop was already requested.
    fn on_wake(&self, f: Box<dyn FnOnce() + Send>) {
        if let Ok(mut w) = self.wake.lock()
            && !self.is_set()
        {
            *w = Some(f);
            return;
        }
        f();
    }
}

pub struct Hotkey {
    on_toggle: Arc<dyn Fn() + Send + Sync>,
    stop: Option<Arc<Stop>>,
}

impl Hotkey {
    pub fn new(on_toggle: Arc<dyn Fn() + Send + Sync>) -> Self {
        Hotkey { on_toggle, stop: None }
    }

    /// Register (`true`) or release (`false`) the shortcut. Cheap to call every frame.
    pub fn set_active(&mut self, active: bool) {
        match (active, &self.stop) {
            (true, None) => {
                let stop = Arc::new(Stop::default());
                self.stop = Some(stop.clone());
                let (toggle, chain) = (self.on_toggle.clone(), env_candidates());
                let spawned = std::thread::Builder::new()
                    .name("overlay-hotkey".into())
                    .spawn(move || worker(&stop, &*toggle, &chain));
                if let Err(e) = spawned {
                    eprintln!("[overlay] could not start the hotkey thread: {e}");
                }
            }
            (false, Some(stop)) => {
                stop.set();
                self.stop = None;
            }
            _ => {}
        }
    }
}

impl Drop for Hotkey {
    fn drop(&mut self) {
        self.set_active(false);
    }
}

/// Try each backend until one runs; returns when stopped.
fn worker(stop: &Stop, toggle: &(dyn Fn() + Send + Sync), chain: &[Backend]) {
    for &backend in chain {
        if stop.is_set() {
            return;
        }
        eprintln!("[overlay] registering Shift+Tab ({backend:?})");
        match run(backend, stop, toggle) {
            Ok(()) => {
                eprintln!("[overlay] released Shift+Tab ({backend:?})");
                return;
            }
            Err(e) => eprintln!("[overlay] Shift+Tab via {backend:?} unavailable: {e}"),
        }
    }
    static WARNED: AtomicBool = AtomicBool::new(false);
    if !stop.is_set() && !WARNED.swap(true, Ordering::SeqCst) {
        eprintln!("[overlay] no global shortcut available; use Games > Show overlay");
    }
}

fn run(backend: Backend, stop: &Stop, toggle: &(dyn Fn() + Send + Sync)) -> Result<(), String> {
    match backend {
        #[cfg(all(unix, not(target_os = "macos")))]
        Backend::Portal => portal::run(stop, toggle),
        #[cfg(all(unix, not(target_os = "macos")))]
        Backend::X11 => x11::run(stop, toggle),
        #[cfg(windows)]
        Backend::Windows => win::run(stop, toggle),
        #[allow(unreachable_patterns)]
        other => {
            let _ = (stop, toggle);
            Err(format!("{other:?} is not available on this system"))
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod portal {
    use super::Stop;
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    use futures_util::future::{Either, select};

    const ID: &str = "toggle-overlay";

    pub fn run(stop: &Stop, toggle: &(dyn Fn() + Send + Sync)) -> Result<(), String> {
        async_io::block_on(async {
            let e = |what: &'static str| move |err: ashpd::Error| format!("{what}: {err}");
            let shortcuts = GlobalShortcuts::new().await.map_err(e("no GlobalShortcuts portal"))?;
            let session = shortcuts.create_session(Default::default()).await.map_err(e("create session"))?;
            let mut activated = shortcuts.receive_activated().await.map_err(e("subscribe"))?;
            let new = NewShortcut::new(ID, "Show or hide the in-game overlay").preferred_trigger("SHIFT+Tab");
            // The first time, the desktop asks the user to confirm; this waits for the answer.
            let bound = shortcuts.bind_shortcuts(&session, &[new], None, Default::default()).await;
            let bound = match bound {
                Ok(req) => req.response().map(|_| ()),
                Err(err) => Err(err),
            };
            if let Err(err) = bound {
                let _ = session.close().await;
                return Err(format!("bind: {err}"));
            }
            let (tx, mut rx) = futures_channel::oneshot::channel::<()>();
            stop.on_wake(Box::new(move || {
                let _ = tx.send(());
            }));
            // Ends when the portal closes the stream, or when we are asked to stop.
            while let Either::Left((Some(a), _)) = select(activated.next(), &mut rx).await {
                if a.shortcut_id() == ID {
                    toggle();
                }
            }
            let _ = session.close().await;
            Ok(())
        })
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod x11 {
    use super::{SHIFT, Stop, keycode_for, mask_variants};
    use std::sync::Arc;
    use x11rb::connection::Connection;
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{
        AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, GrabMode, ModMask, WindowClass,
    };
    use x11rb::rust_connection::RustConnection;

    /// XK_Tab
    const KEYSYM_TAB: u32 = 0xff09;

    pub fn run(stop: &Stop, toggle: &(dyn Fn() + Send + Sync)) -> Result<(), String> {
        let (conn, screen) = RustConnection::connect(None).map_err(|e| format!("connect: {e}"))?;
        let conn = Arc::new(conn);
        let setup = conn.setup();
        let root = setup.roots[screen].root;
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let map = conn
            .get_keyboard_mapping(min, max - min + 1)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?;
        let keycode = keycode_for(min, usize::from(map.keysyms_per_keycode), &map.keysyms, KEYSYM_TAB)
            .ok_or("no Tab key on this keyboard")?;
        let ungrab = {
            let conn = conn.clone();
            move || {
                for m in mask_variants(SHIFT) {
                    let _ = conn.ungrab_key(keycode, root, ModMask::from(m));
                }
                let _ = conn.flush();
            }
        };
        for m in mask_variants(SHIFT) {
            let cookie = conn
                .grab_key(false, root, ModMask::from(m), keycode, GrabMode::ASYNC, GrabMode::ASYNC)
                .map_err(|e| e.to_string())?;
            if let Err(e) = cookie.check() {
                ungrab();
                return Err(format!("another program holds Shift+Tab ({e})"));
            }
        }
        // A window of our own to send a wake-up event to, so the blocking read below can end.
        let win = conn.generate_id().map_err(|e| e.to_string())?;
        conn.create_window(0, win, root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())
            .map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        let waker = conn.clone();
        let unlock = ungrab.clone();
        stop.on_wake(Box::new(move || {
            unlock();
            let ev = ClientMessageEvent::new(32, win, AtomEnum::STRING, [0u32; 5]);
            let _ = waker.send_event(false, win, EventMask::NO_EVENT, ev);
            let _ = waker.flush();
        }));
        while !stop.is_set() {
            match conn.wait_for_event() {
                Ok(Event::KeyPress(k)) if k.detail == keycode => toggle(),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        ungrab();
        let _ = conn.destroy_window(win);
        let _ = conn.flush();
        Ok(())
    }
}

#[cfg(windows)]
mod win {
    use super::Stop;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey, VK_TAB,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetMessageW, MSG, PM_NOREMOVE, PeekMessageW, PostThreadMessageW, WM_HOTKEY, WM_QUIT,
    };

    const ID: i32 = 0x5352;

    pub fn run(stop: &Stop, toggle: &(dyn Fn() + Send + Sync)) -> Result<(), String> {
        // SAFETY: plain Win32 calls on this thread; `msg` outlives each call.
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            // Make sure this thread has a message queue before anyone posts to it.
            PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
            if RegisterHotKey(std::ptr::null_mut(), ID, MOD_SHIFT | MOD_NOREPEAT, u32::from(VK_TAB)) == 0 {
                return Err("Shift+Tab is taken by another program".into());
            }
            let tid = GetCurrentThreadId();
            stop.on_wake(Box::new(move || {
                // Posting a message to a thread id is memory safe.
                PostThreadMessageW(tid, WM_QUIT, 0, 0);
            }));
            while !stop.is_set() && GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                if msg.message == WM_HOTKEY {
                    toggle();
                }
            }
            UnregisterHotKey(std::ptr::null_mut(), ID);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(windows))]
    fn backend_from_environment() {
        use Backend::*;
        assert_eq!(candidates(Some("wayland-0"), Some("wayland"), Some(":0")), vec![Portal, X11]);
        assert_eq!(candidates(Some("wayland-0"), None, None), vec![Portal]);
        assert_eq!(candidates(None, Some("Wayland"), None), vec![Portal]);
        assert_eq!(candidates(None, Some("x11"), Some(":58")), vec![X11]);
        assert_eq!(candidates(Some(""), None, Some(":0")), vec![X11]);
        assert!(candidates(None, None, None).is_empty());
    }

    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn caps_and_num_lock_variants() {
        let v = mask_variants(SHIFT);
        assert_eq!(v, [1, 3, 17, 19]);
        assert!(v.iter().all(|m| m & SHIFT != 0));
        let mut sorted = v;
        sorted.sort_unstable();
        sorted.windows(2).for_each(|w| assert_ne!(w[0], w[1]));
    }

    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn tab_keycode_lookup() {
        // min keycode 8, two keysyms per key: key 8 = (a, A), key 9 = (Tab, ISO_Left_Tab).
        let map = [0x61, 0x41, 0xff09, 0xfe20];
        assert_eq!(keycode_for(8, 2, &map, 0xff09), Some(9));
        assert_eq!(keycode_for(8, 2, &map, 0x7a), None);
        assert_eq!(keycode_for(8, 0, &map, 0xff09), None);
    }

    #[test]
    fn stop_runs_wake_once_even_when_late() {
        use std::sync::atomic::AtomicUsize;
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Stop::default();
        let h = hits.clone();
        stop.on_wake(Box::new(move || {
            h.fetch_add(1, Ordering::SeqCst);
        }));
        stop.set();
        stop.set();
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let h = hits.clone();
        stop.on_wake(Box::new(move || {
            h.fetch_add(1, Ordering::SeqCst);
        }));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }
}
