//! A bug in the overlay must never kill the game. Every function the library exports to the game (the GL swap
//! hooks, the Vulkan layer entries, the SDL and Xlib input hooks, `dlsym`) runs its own code through [`run`]:
//! a panic is caught, logged once, the overlay is switched off for the rest of the process and the call goes
//! straight through to the real function. After that [`off`] is true and the hooks do nothing but pass through.
//!
//! This needs unwinding panics: Cargo ignores a per-package `panic` setting, so the workspace profiles must not
//! set `panic = "abort"` (see the root `Cargo.toml`). The test `panics_are_caught` aborts when they do.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

static OFF: AtomicBool = AtomicBool::new(false);

/// The overlay was switched off for this process (after a panic).
pub fn off() -> bool {
    OFF.load(Ordering::Acquire)
}

/// Switch the overlay off for good: closed, no drawing, no input takeover. The first call logs.
pub fn disable(why: &str) {
    if OFF.swap(true, Ordering::AcqRel) {
        return;
    }
    eprintln!("[kettle-overlay] disabled for this process: {why}");
    crate::ipc::log(&format!("disabled {why}"));
    // Give the game its mouse, keyboard and cursor back when it was open (this may panic too: ignore).
    let _ = catch_unwind(AssertUnwindSafe(crate::state::force_closed));
}

/// Run `f`; `None` when it panicked (the overlay is then off) or the overlay already is.
pub fn run<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    if off() {
        return None;
    }
    run_always(what, f)
}

/// Like [`run`], but also when the overlay is off: for bookkeeping the game's Vulkan calls still depend on.
pub fn run_always<R>(what: &str, f: impl FnOnce() -> R) -> Option<R> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => Some(r),
        Err(e) => {
            let msg = e
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| e.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "panic".into());
            disable(&format!("{what}: {msg}"));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panics_are_caught() {
        assert_eq!(run("test", || 7), Some(7));
        let r = run("test", || -> u8 { panic!("boom") });
        assert_eq!(r, None);
        assert!(off());
        // Off: nothing runs any more.
        assert_eq!(run("test", || 1), None);
    }
}
