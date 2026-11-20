//! Opening a URL in the system browser. The one place that does it.
//! Payments are never handled in the app: "Buy" just opens the store page.

use anyhow::{Result, bail};
use std::process::{Command, Stdio};

/// Only Valve's store is opened from the UI; anything else is refused.
const ALLOWED_PREFIX: &str = "https://store.steampowered.com/";

/// Start the browser and return at once. A thread reaps the child so no
/// zombie is left behind; the UI thread never waits.
pub fn url(url: &str) -> Result<()> {
    if !url.starts_with(ALLOWED_PREFIX) {
        bail!("refusing to open {url}");
    }
    let mut child = launcher(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
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
}
