//! The text of Help > System information. Reads a few small files and environment variables.

use crate::{Load, State};
use sr_steam::local;

fn os_release_name() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string())))
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

fn kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default()
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

/// A plain-text report the user can copy into a bug report. `gpu` is the graphics adapter name the window uses.
pub fn system_info(state: &State, gpu: &str) -> String {
    let roots = local::steam_roots();
    let protons = match &state.compat.info {
        Load::Ready(info) => info.tools.len().to_string(),
        _ => "not loaded yet".to_string(),
    };
    let mut out = String::new();
    let mut line = |k: &str, v: String| out.push_str(&format!("{k}: {v}\n"));
    line("Kettle", env!("CARGO_PKG_VERSION").to_string());
    line("OS", os_release_name());
    line("Kernel", kernel());
    line("Desktop", env("XDG_CURRENT_DESKTOP"));
    line("Session", env("XDG_SESSION_TYPE"));
    line("Graphics adapter", if gpu.is_empty() { "unknown".into() } else { gpu.to_string() });
    line("Steam folders found", roots.len().to_string());
    for r in &roots {
        line("  ", r.display().to_string());
    }
    line("Proton tools", protons);
    out
}
