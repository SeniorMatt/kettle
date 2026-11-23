//! The in-game overlay library (`libkettle_overlay.so`, crate `sr-overlay-hook`): where it lives, the Vulkan
//! layer manifest, and the launch variables that switch it on for a game.
//!
//! The library is copied to `<data dir>/overlay/` so it has one stable path under `$HOME` (which the Steam
//! Linux Runtime container shares, as it does for MangoHud's layer). The Vulkan implicit layer manifest goes to
//! `$XDG_DATA_HOME/vulkan/implicit_layer.d/` (default `~/.local/share/...`), a folder the runtime container
//! imports. The layer only activates with `KETTLE_OVERLAY=1`, which only our launches set.

use crate::run::{LaunchPlan, Runner};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// The installed library and Kettle's socket, as a launch needs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hook {
    pub lib: PathBuf,
    /// The 32-bit copy, for 32-bit games (preloaded too: the loader skips the copy of the wrong word size).
    pub lib32: Option<PathBuf>,
    pub socket: Option<PathBuf>,
}

pub const LIB_NAME: &str = "libkettle_overlay.so";
pub const LIB32_NAME: &str = "libkettle_overlay32.so";
pub const LAYER_FILE: &str = "kettle_overlay.json";
/// The Vulkan layer manifest of the 32-bit copy (same layer name: a loader uses the one it can load).
pub const LAYER_FILE32: &str = "kettle_overlay_32.json";
/// The folder the library is installed to.
pub fn install_dir() -> PathBuf {
    crate::store::data_dir().join("overlay")
}

/// Where the installed library is.
pub fn installed_lib() -> PathBuf {
    install_dir().join(LIB_NAME)
}

/// Where the installed 32-bit library is (when there is one).
pub fn installed_lib32() -> PathBuf {
    install_dir().join(LIB32_NAME)
}

/// `$XDG_DATA_HOME/vulkan/implicit_layer.d` (or `~/.local/share/vulkan/implicit_layer.d`).
pub fn layer_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").filter(|v| !v.is_empty())?).join(".local/share"),
    };
    Some(base.join("vulkan/implicit_layer.d"))
}

/// The Vulkan layer manifest for a library at `lib`.
pub fn manifest(lib: &Path) -> String {
    format!(
        r#"{{
  "file_format_version": "1.0.0",
  "layer": {{
    "name": "VK_LAYER_KETTLE_overlay",
    "type": "GLOBAL",
    "library_path": {lib},
    "api_version": "1.3.0",
    "implementation_version": "1",
    "description": "Kettle in-game overlay",
    "functions": {{
      "vkNegotiateLoaderLayerInterfaceVersion": "kettle_vk_negotiate",
      "vkGetInstanceProcAddr": "kettle_vk_gipa",
      "vkGetDeviceProcAddr": "kettle_vk_gdpa"
    }},
    "enable_environment": {{ "KETTLE_OVERLAY": "1" }},
    "disable_environment": {{ "KETTLE_OVERLAY_DISABLE": "1" }}
  }}
}}
"#,
        lib = serde_json_string(&lib.to_string_lossy())
    )
}

fn serde_json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Where a built library may be found next to the running program: `<exe dir>/../lib/` (AppImage) or beside the
/// program (a cargo target folder).
pub fn find_source(exe: &Path) -> Option<PathBuf> {
    find_named(exe, LIB_NAME)
}

/// The same for the 32-bit copy.
pub fn find_source32(exe: &Path) -> Option<PathBuf> {
    find_named(exe, LIB32_NAME)
}

fn find_named(exe: &Path, name: &str) -> Option<PathBuf> {
    let dir = exe.parent()?;
    [dir.join("../lib").join(name), dir.join(name)].into_iter().find(|p| p.is_file())
}

/// Copy `source` to the data folder (when it differs) and write the layer manifest. Returns the installed path.
pub fn install(source: &Path) -> Result<PathBuf> {
    install_as(source, &installed_lib(), LAYER_FILE)
}

/// The same for the 32-bit copy.
pub fn install32(source: &Path) -> Result<PathBuf> {
    install_as(source, &installed_lib32(), LAYER_FILE32)
}

fn install_as(source: &Path, dest: &Path, layer_file: &str) -> Result<PathBuf> {
    let dest = dest.to_path_buf();
    let dir = install_dir();
    crate::store::private_dir(&dir)?;
    let (a, b) = (std::fs::metadata(source)?, std::fs::metadata(&dest).ok());
    let same = b.as_ref().is_some_and(|b| b.len() == a.len() && b.modified().ok() >= a.modified().ok());
    if !same {
        // Write beside, then rename: a game that has the old library mapped keeps it.
        let tmp = dir.join(format!("{}.new", dest.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()));
        std::fs::copy(source, &tmp).with_context(|| format!("copying {}", source.display()))?;
        std::fs::rename(&tmp, &dest)?;
    }
    if let Some(ld) = layer_dir() {
        std::fs::create_dir_all(&ld)?;
        let want = manifest(&dest);
        let file = ld.join(layer_file);
        if std::fs::read_to_string(&file).ok().as_deref() != Some(want.as_str()) {
            std::fs::write(&file, want)?;
        }
    }
    Ok(dest)
}

/// Switch the overlay on for a launch: the library is preloaded (appended to any `LD_PRELOAD` already set; 64-bit
/// only for now, a 32-bit game gets no overlay), the Vulkan layer is enabled, and Kettle's socket is named.
/// The Steam Linux Runtime container gets the library folder as a shared path.
pub fn apply(plan: &mut LaunchPlan, lib: &Path, lib32: Option<&Path>, socket: Option<&Path>) {
    if matches!(plan.runner, Runner::Direct) {
        return;
    }
    // Both word sizes: the dynamic loader skips the one that does not fit the process (with a harmless message
    // on stderr), so 32-bit games get the 32-bit copy and everything else the 64-bit one.
    for l in std::iter::once(lib).chain(lib32) {
        let l = l.to_string_lossy().into_owned();
        let pre = plan.env.entry("LD_PRELOAD".into()).or_default();
        if !pre.split([':', ' ']).any(|p| p == l) {
            if !pre.is_empty() {
                pre.push(':');
            }
            pre.push_str(&l);
        }
    }
    plan.env.insert("KETTLE_OVERLAY".into(), "1".into());
    if let Some(s) = socket {
        plan.env.insert("KETTLE_OVERLAY_SOCKET".into(), s.to_string_lossy().into_owned());
    }
    if let Some(dir) = lib.parent() {
        let d = dir.to_string_lossy();
        let mounts = plan.env.entry("PRESSURE_VESSEL_FILESYSTEMS_RO".into()).or_default();
        if !mounts.split(':').any(|m| m == d) {
            if !mounts.is_empty() {
                mounts.push(':');
            }
            mounts.push_str(&d);
        }
    }
}

/// The variables only the game itself may get, taken out of `plan.env` for a launch that goes through a wrapper
/// (Gamescope) which must not load the overlay itself: `KETTLE_OVERLAY*`, and `LD_PRELOAD` when it names our
/// libraries (the full value moves to the game; the wrapper keeps what else was in it). Returns them for `env`.
pub fn take_for_child(plan: &mut LaunchPlan, lib: &Path, lib32: Option<&Path>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for k in ["KETTLE_OVERLAY", "KETTLE_OVERLAY_SOCKET"] {
        if let Some(v) = plan.env.remove(k) {
            out.push((k.to_string(), v));
        }
    }
    if let Some(all) = plan.env.get("LD_PRELOAD").cloned() {
        let ours: Vec<String> = std::iter::once(lib).chain(lib32).map(|p| p.to_string_lossy().into_owned()).collect();
        let items: Vec<&str> = all.split([':', ' ']).filter(|p| !p.is_empty()).collect();
        let others: Vec<&str> = items.iter().copied().filter(|p| !ours.iter().any(|o| o == p)).collect();
        if others.len() != items.len() {
            out.push(("LD_PRELOAD".to_string(), all.clone()));
            if others.is_empty() {
                plan.env.remove("LD_PRELOAD");
            } else {
                plan.env.insert("LD_PRELOAD".into(), others.join(":"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn plan(runner: Runner) -> LaunchPlan {
        LaunchPlan {
            appid: 1,
            runner,
            program: PathBuf::from("/g/run"),
            args: vec![],
            cwd: PathBuf::from("/g"),
            env: BTreeMap::new(),
            create_dirs: vec![],
        }
    }

    #[test]
    fn preload_is_appended_not_replaced() {
        let mut p = plan(Runner::Native);
        p.env.insert("LD_PRELOAD".into(), "/x/other.so".into());
        let (l64, l32) = (Path::new("/d/overlay/libkettle_overlay.so"), Path::new("/d/overlay/libkettle_overlay32.so"));
        apply(&mut p, l64, Some(l32), Some(Path::new("/run/u/k.sock")));
        assert_eq!(
            p.env["LD_PRELOAD"],
            "/x/other.so:/d/overlay/libkettle_overlay.so:/d/overlay/libkettle_overlay32.so"
        );
        assert_eq!(p.env["KETTLE_OVERLAY"], "1");
        assert_eq!(p.env["KETTLE_OVERLAY_SOCKET"], "/run/u/k.sock");
        assert_eq!(p.env["PRESSURE_VESSEL_FILESYSTEMS_RO"], "/d/overlay");
        // twice is the same
        apply(&mut p, l64, Some(l32), None);
        assert_eq!(
            p.env["LD_PRELOAD"],
            "/x/other.so:/d/overlay/libkettle_overlay.so:/d/overlay/libkettle_overlay32.so"
        );
        // Without a 32-bit copy only the 64-bit one is named.
        let mut q = plan(Runner::Native);
        apply(&mut q, l64, None, None);
        assert_eq!(q.env["LD_PRELOAD"], "/d/overlay/libkettle_overlay.so");
    }

    #[test]
    fn a_wrapper_keeps_only_what_is_not_ours() {
        let mut p = plan(Runner::Native);
        p.env.insert("LD_PRELOAD".into(), "/x/other.so".into());
        let (l64, l32) = (Path::new("/d/o/a.so"), Path::new("/d/o/b.so"));
        apply(&mut p, l64, Some(l32), Some(Path::new("/run/k.sock")));
        let child = take_for_child(&mut p, l64, Some(l32));
        assert_eq!(p.env.get("LD_PRELOAD").map(String::as_str), Some("/x/other.so"));
        assert!(!p.env.contains_key("KETTLE_OVERLAY") && !p.env.contains_key("KETTLE_OVERLAY_SOCKET"));
        assert!(child.contains(&("LD_PRELOAD".to_string(), "/x/other.so:/d/o/a.so:/d/o/b.so".to_string())));
        assert!(child.contains(&("KETTLE_OVERLAY".to_string(), "1".to_string())));
        assert!(child.contains(&("KETTLE_OVERLAY_SOCKET".to_string(), "/run/k.sock".to_string())));
        // Nothing else in LD_PRELOAD: the variable leaves the wrapper's environment.
        let mut q = plan(Runner::Native);
        apply(&mut q, l64, None, None);
        take_for_child(&mut q, l64, None);
        assert!(!q.env.contains_key("LD_PRELOAD"));
    }

    #[test]
    fn windows_direct_gets_nothing() {
        let mut p = plan(Runner::Direct);
        apply(&mut p, Path::new("/d/l.so"), None, None);
        assert!(p.env.is_empty());
    }

    #[test]
    fn manifest_names_the_layer_and_the_library() {
        let m = manifest(Path::new("/home/a b/kettle/overlay/libkettle_overlay.so"));
        assert!(m.contains("VK_LAYER_KETTLE_overlay"));
        assert!(m.contains(r#""library_path": "/home/a b/kettle/overlay/libkettle_overlay.so""#));
        assert!(m.contains("KETTLE_OVERLAY"));
    }
}
