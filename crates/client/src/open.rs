//! Opening a URL in the system browser. The one place that does it.
//! Payments are never handled in the app: "Buy" just opens the store page.

use anyhow::{Result, bail};
use std::path::Path;
use std::process::{Command, Stdio};

/// Only Valve's store is opened from the UI; anything else is refused.
const ALLOWED_PREFIXES: [&str; 3] =
    ["https://store.steampowered.com/", "https://help.steampowered.com/", "https://github.com/SeniorMatt/kettle"];

/// Start the browser and return at once. A thread reaps the child so no
/// zombie is left behind; the UI thread never waits.
pub fn url(url: &str) -> Result<()> {
    if !ALLOWED_PREFIXES.iter().any(|p| url.starts_with(p)) {
        bail!("refusing to open {url}");
    }
    let mut child = launcher(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Open a game's install folder in the file manager. Only folders in `allowed` (the install folders of
/// installed games) are opened, compared after resolving symlinks; anything else is refused.
pub fn folder(dir: &Path, allowed: &[&Path]) -> Result<()> {
    let real = dir.canonicalize()?;
    if !real.is_dir() || !allowed.iter().any(|a| a.canonicalize().is_ok_and(|a| a == real)) {
        bail!("refusing to open {}", dir.display());
    }
    let mut child = folder_launcher(&real).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(target_os = "windows")]
fn folder_launcher(dir: &Path) -> Command {
    let mut c = Command::new("explorer");
    c.arg(dir);
    c
}

#[cfg(not(target_os = "windows"))]
fn folder_launcher(dir: &Path) -> Command {
    // Not a URL and always absolute, so it can't be taken for an option.
    #[cfg(target_os = "macos")]
    let mut c = Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut c = Command::new("xdg-open");
    c.arg(dir);
    c
}

#[cfg(target_os = "windows")]
fn launcher(url: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("cmd");
    // CREATE_NO_WINDOW. `start` takes a window title first; the empty "" is it.
    c.args(["/c", "start", "", url]).creation_flags(0x0800_0000);
    c
}

#[cfg(target_os = "macos")]
fn launcher(url: &str) -> Command {
    let mut c = Command::new("open");
    c.arg(url);
    c
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn launcher(url: &str) -> Command {
    let mut c = Command::new("xdg-open");
    c.arg(url);
    c
}

#[cfg(test)]
mod tests {
    #[test]
    fn refuses_foreign_urls() {
        assert!(super::url("https://evil.example/").is_err());
        assert!(super::url("file:///etc/passwd").is_err());
    }

    #[test]
    fn folders_must_be_installed_games() {
        let tmp = std::env::temp_dir();
        assert!(super::folder(&tmp, &[]).is_err());
        assert!(super::folder(&tmp, &[std::path::Path::new("/etc")]).is_err());
        assert!(
            super::folder(std::path::Path::new("/nonexistent/x"), &[std::path::Path::new("/nonexistent/x")]).is_err()
        );
    }
}
