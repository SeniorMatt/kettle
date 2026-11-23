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

pub const LIB_NAME: &str = "libkettle_overlay.so";
pub const LAYER_FILE: &str = "kettle_overlay.json";
/// The folder the library is installed to.
pub fn install_dir() -> PathBuf {
    crate::store::data_dir().join("overlay")
}

/// Where the installed library is.
pub fn installed_lib() -> PathBuf {
    install_dir().join(LIB_NAME)
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
    let dir = exe.parent()?;
    [dir.join("../lib").join(LIB_NAME), dir.join(LIB_NAME)].into_iter().find(|p| p.is_file())
}

/// Copy `source` to the data folder (when it differs) and write the layer manifest. Returns the installed path.
pub fn install(source: &Path) -> Result<PathBuf> {
    let dest = installed_lib();
    let dir = install_dir();
    crate::store::private_dir(&dir)?;
    let (a, b) = (std::fs::metadata(source)?, std::fs::metadata(&dest).ok());
    let same = b.as_ref().is_some_and(|b| b.len() == a.len() && b.modified().ok() >= a.modified().ok());
    if !same {
        // Write beside, then rename: a game that has the old library mapped keeps it.
        let tmp = dir.join(format!("{LIB_NAME}.new"));
        std::fs::copy(source, &tmp).with_context(|| format!("copying {}", source.display()))?;
        std::fs::rename(&tmp, &dest)?;
    }
    if let Some(ld) = layer_dir() {
        std::fs::create_dir_all(&ld)?;
        let want = manifest(&dest);
        let file = ld.join(LAYER_FILE);
        if std::fs::read_to_string(&file).ok().as_deref() != Some(want.as_str()) {
            std::fs::write(&file, want)?;
        }
    }
    Ok(dest)
}

/// Switch the overlay on for a launch: the library is preloaded (appended to any `LD_PRELOAD` already set; 64-bit
/// only for now, a 32-bit game gets no overlay), the Vulkan layer is enabled, and Kettle's socket is named.
/// The Steam Linux Runtime container gets the library folder as a shared path.
pub fn apply(plan: &mut LaunchPlan, lib: &Path, socket: Option<&Path>) {
    if matches!(plan.runner, Runner::Direct) {
        return;
    }
    let l = lib.to_string_lossy().into_owned();
    let pre = plan.env.entry("LD_PRELOAD".into()).or_default();
    if !pre.split([':', ' ']).any(|p| p == l) {
        if !pre.is_empty() {
            pre.push(':');
        }
        pre.push_str(&l);
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
        apply(&mut p, Path::new("/d/overlay/libkettle_overlay.so"), Some(Path::new("/run/u/k.sock")));
        assert_eq!(p.env["LD_PRELOAD"], "/x/other.so:/d/overlay/libkettle_overlay.so");
        assert_eq!(p.env["KETTLE_OVERLAY"], "1");
        assert_eq!(p.env["KETTLE_OVERLAY_SOCKET"], "/run/u/k.sock");
        assert_eq!(p.env["PRESSURE_VESSEL_FILESYSTEMS_RO"], "/d/overlay");
        // twice is the same
        apply(&mut p, Path::new("/d/overlay/libkettle_overlay.so"), None);
        assert_eq!(p.env["LD_PRELOAD"], "/x/other.so:/d/overlay/libkettle_overlay.so");
    }

    #[test]
    fn windows_direct_gets_nothing() {
        let mut p = plan(Runner::Direct);
        apply(&mut p, Path::new("/d/l.so"), None);
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
