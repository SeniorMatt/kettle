//! "Start with the computer": an XDG autostart entry, `~/.config/autostart/kettle.desktop`.
//!
//! The file is the truth: the setting shows whether it exists. It starts the AppImage (`$APPIMAGE`) or, outside
//! one, the running program, with `--minimized` when "Start minimized" is on (the window then stays hidden in the
//! tray, see `crates/app`).

use super::Client;
use super::shortcuts::exec_quote;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "kettle.desktop";
/// The command-line flag that starts with the window hidden.
pub const MINIMIZED_ARG: &str = "--minimized";

/// `$XDG_CONFIG_HOME/autostart` or `~/.config/autostart`.
pub fn autostart_dir() -> Option<PathBuf> {
    let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(std::env::var_os("HOME").filter(|h| !h.is_empty())?).join(".config"),
    };
    base.is_absolute().then(|| base.join("autostart"))
}

pub fn file_path() -> Option<PathBuf> {
    autostart_dir().map(|d| d.join(FILE_NAME))
}

/// What the entry starts: the AppImage when there is one, else this program.
pub fn exe_path() -> Option<PathBuf> {
    crate::kettle_update::appimage_path().or_else(|| std::env::current_exe().ok())
}

/// The text of the entry.
pub fn entry_text(exe: &Path, minimized: bool) -> String {
    let mut exec = exec_quote(&exe.to_string_lossy());
    if minimized {
        exec.push(' ');
        exec.push_str(MINIMIZED_ARG);
    }
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Kettle\nComment=Start Kettle with the computer\n\
         Exec={exec}\nIcon=kettle\nTerminal=false\nCategories=Game;\nStartupWMClass=kettle\n\
         X-GNOME-Autostart-enabled=true\n"
    )
}

/// Is the autostart file there?
pub fn is_enabled() -> bool {
    file_path().is_some_and(|p| p.is_file())
}

/// Write or remove the entry.
pub fn set_enabled(on: bool, minimized: bool) -> Result<()> {
    let path = file_path().context("can't find the autostart folder (HOME is not set)")?;
    if on {
        let exe = exe_path().context("can't tell which program to start")?;
        write_entry(&path, &exe, minimized)
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
        }
    }
}

fn write_entry(path: &Path, exe: &Path, minimized: bool) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(path, entry_text(exe, minimized)).with_context(|| format!("writing {}", path.display()))
}

impl Client {
    /// `Command::SetAutostart`.
    pub(crate) fn set_autostart(&mut self, on: bool) {
        match set_enabled(on, self.state.settings.interface.start_minimized) {
            Ok(()) => self.state.autostart = is_enabled(),
            Err(e) => {
                self.state.autostart = is_enabled();
                self.state.notice = Some(format!("Couldn't change \"Start with the computer\": {e:#}"));
            }
        }
    }

    /// "Start minimized" changed: keep an existing entry in step with it.
    pub(crate) fn refresh_autostart(&mut self) {
        if self.state.autostart {
            self.set_autostart(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_text_has_the_flag_only_when_asked() {
        let t = entry_text(Path::new("/opt/My Apps/kettle.AppImage"), false);
        assert!(t.contains("\nExec=\"/opt/My Apps/kettle.AppImage\"\n"));
        let t = entry_text(Path::new("/opt/kettle.AppImage"), true);
        assert!(t.contains("\nExec=\"/opt/kettle.AppImage\" --minimized\n"));
        assert!(t.starts_with("[Desktop Entry]\n"));
        assert_eq!(t.matches("\nExec=").count(), 1);
    }

    #[test]
    fn writes_and_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("kettle-autostart-{}", std::process::id()));
        let path = dir.join("autostart").join(FILE_NAME);
        write_entry(&path, Path::new("/x/kettle"), true).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("--minimized"));
        // Writing again replaces it.
        write_entry(&path, Path::new("/x/kettle"), false).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("--minimized"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
