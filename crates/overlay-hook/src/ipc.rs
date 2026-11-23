//! The connection to Kettle: data in, actions out. One background thread, started with the overlay. A game
//! without Kettle (no `KETTLE_OVERLAY_SOCKET`) simply has no connection and the overlay shows what it has.

use crate::state;
use sr_overlay_proto::{FromHook, MAX_LINE, OverlayAction, ToHook, decode, encode};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn out() -> &'static Mutex<Option<UnixStream>> {
    static OUT: OnceLock<Mutex<Option<UnixStream>>> = OnceLock::new();
    OUT.get_or_init(|| Mutex::new(None))
}

/// Send one message; dropped silently when not connected or when Kettle does not read (a stopped or hung Kettle
/// must never stall the game's frame). A broken connection is forgotten; the connect loop makes a new one.
pub fn send(msg: &FromHook) {
    use std::os::fd::AsRawFd;
    let line = encode(msg);
    let Ok(mut g) = out().try_lock() else { return };
    if let Some(s) = g.as_mut() {
        // MSG_NOSIGNAL: the game is not ours, so SIGPIPE would kill it. MSG_DONTWAIT: never block.
        let n = unsafe {
            libc::send(s.as_raw_fd(), line.as_ptr().cast(), line.len(), libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT)
        };
        if n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock {
            *g = None;
        }
    }
}

/// The user did something. Also appended to `$KETTLE_OVERLAY_LOG` when set (the tests read it).
pub fn action(a: &OverlayAction) {
    if let Some(path) = std::env::var_os("KETTLE_OVERLAY_LOG")
        && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(f, "action {}", String::from_utf8_lossy(&encode(a)).trim_end());
    }
    send(&FromHook::Action { a: a.clone() });
}

/// Log a line to `$KETTLE_OVERLAY_LOG` (tests only; does nothing without the variable).
pub fn log(line: &str) {
    if let Some(path) = std::env::var_os("KETTLE_OVERLAY_LOG")
        && let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

pub fn start() {
    let Some(path) = std::env::var_os("KETTLE_OVERLAY_SOCKET").filter(|p| !p.is_empty()) else { return };
    let _ = std::thread::Builder::new().name("kettle-overlay".into()).spawn(move || {
        loop {
            // A panic ends only this connection, never the thread or the game; the overlay keeps showing the
            // last data it got while Kettle is away, and the loop connects again.
            let _ = std::panic::catch_unwind(|| connect_once(&path));
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

fn connect_once(path: &std::ffi::OsStr) {
    if let Ok(stream) = UnixStream::connect(path) {
        if let Ok(w) = stream.try_clone() {
            if let Ok(mut g) = out().lock() {
                *g = Some(w);
            }
            let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
            send(&FromHook::Hello { pid: std::process::id(), exe });
            read_loop(stream);
        }
        if let Ok(mut g) = out().lock() {
            *g = None;
        }
    }
}

fn read_loop(stream: UnixStream) {
    let mut r = BufReader::new(stream);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.by_ref().take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) if buf.len() > MAX_LINE => return,
            Ok(_) => {}
        }
        match decode::<ToHook>(&buf) {
            Some(ToHook::Data(d)) => state::set_data(d),
            Some(ToHook::Toggle) => state::toggle(),
            Some(ToHook::Avatar { id, size, rgba }) => {
                if let Some(px) = sr_overlay_proto::b64_decode(&rgba) {
                    state::set_avatar(id, size, px);
                }
            }
            None => {}
        }
    }
}
