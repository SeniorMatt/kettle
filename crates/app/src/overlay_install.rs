//! Puts the in-game overlay library where games can load it (see `sr_steam::overlay_hook`).

use crate::overlay_host::Host;
use sr_steam::overlay_hook::Hook;

/// Install `libkettle_overlay.so` and the Vulkan layer manifest; the hook to start games with, or `None` when the
/// library is not shipped next to this program (a plain `cargo run` without building it) or cannot be installed.
pub fn prepare(host: &mut Host) -> Option<Hook> {
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    {
        let _ = host;
        None
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use sr_steam::overlay_hook;
        let exe = std::env::current_exe().ok()?;
        // Next to the program: `../lib/` in the AppImage, the cargo target folder in a development build.
        let source = overlay_hook::find_source(&exe).or_else(|| {
            let p = overlay_hook::installed_lib();
            p.is_file().then_some(p)
        })?;
        let lib = match overlay_hook::install(&source) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[overlay] cannot install the overlay library: {e:#}");
                return None;
            }
        };
        Some(Hook { lib, socket: host.socket() })
    }
}
