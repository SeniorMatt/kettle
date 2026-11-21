//! Desktop and application-menu shortcuts: `.desktop` files that start a game with `kettle --play <appid>`.
//!
//! Files written (only when the user asks, from the install dialog or the game's menu):
//! - `<applications dir>/kettle-game-<appid>.desktop`, the application menu entry (`~/.local/share/applications`)
//! - `<desktop dir>/kettle-game-<appid>.desktop`, the desktop icon (`~/Desktop`, or the folder named in
//!   `~/.config/user-dirs.dirs`)
//!
//! They are removed again when the game is uninstalled.

use anyhow::{Result, bail};
use std::path::{Path, PathBuf};

pub fn file_name(appid: u32) -> String {
    format!("kettle-game-{appid}.desktop")
}

/// Quote one argument for an `Exec=` line (desktop entry spec): double quotes, with `"`, `` ` ``, `$` and `\`
/// escaped, and `%` doubled so it isn't read as a field code.
pub fn exec_quote(arg: &str) -> String {
    let mut out = String::from("\"");
    for c in arg.chars() {
        match c {
            '"' | '`' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '%' => out.push_str("%%"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The text of the `.desktop` file. `icon` is an icon name or an absolute path.
pub fn desktop_entry(name: &str, appid: u32, exe: &str, icon: &str) -> String {
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName={name}\nComment=Play {name} with Kettle\n\
         Exec={} --play {appid}\nIcon={icon}\nTerminal=false\nCategories=Game;\nStartupWMClass=kettle\n",
        exec_quote(exe)
    )
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|h| !h.is_empty()).map(PathBuf::from)
}

pub fn applications_dir() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(d) => Some(PathBuf::from(d).join("applications")),
        None => Some(home()?.join(".local/share/applications")),
    }
}

/// The folder for desktop icons: `XDG_DESKTOP_DIR` from `~/.config/user-dirs.dirs`, else `~/Desktop`.
pub fn desktop_dir() -> Option<PathBuf> {
    let home = home()?;
    let dirs = std::fs::read_to_string(home.join(".config/user-dirs.dirs")).unwrap_or_default();
    Some(parse_desktop_dir(&dirs, &home).unwrap_or_else(|| home.join("Desktop")))
}

/// Find `XDG_DESKTOP_DIR="$HOME/Desktop"` in the text of `user-dirs.dirs`.
pub fn parse_desktop_dir(text: &str, home: &Path) -> Option<PathBuf> {
    let line = text.lines().map(str::trim).find(|l| l.starts_with("XDG_DESKTOP_DIR="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let path = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(value),
    };
    (path.is_absolute() && path != home).then_some(path)
}

/// Write the chosen shortcuts and return the files made.
pub fn create(name: &str, appid: u32, desktop: bool, menu: bool, icon: Option<&Path>) -> Result<Vec<PathBuf>> {
    let exe = std::env::current_exe()?;
    let icon = icon.map_or_else(|| "kettle".to_string(), |p| p.display().to_string());
    let text = desktop_entry(name, appid, &exe.display().to_string(), &icon);
    let mut targets = Vec::new();
    if menu {
        match applications_dir() {
            Some(d) => targets.push(d),
            None => bail!("can't find the applications folder (HOME is not set)"),
        }
    }
    if desktop {
        match desktop_dir() {
            Some(d) => targets.push(d),
            None => bail!("can't find the desktop folder (HOME is not set)"),
        }
    }
    let mut made = Vec::new();
    for dir in targets {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(file_name(appid));
        std::fs::write(&path, &text)?;
        make_executable(&path);
        made.push(path);
    }
    Ok(made)
}

/// Delete both shortcuts of a game if they exist.
pub fn remove(appid: u32) {
    for dir in [applications_dir(), desktop_dir()].into_iter().flatten() {
        let _ = std::fs::remove_file(dir.join(file_name(appid)));
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn make_executable(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_entry_text() {
        let t = desktop_entry("Half-Life 2", 220, "/opt/kettle app/kettle", "kettle");
        assert!(t.starts_with("[Desktop Entry]\n"));
        assert!(t.contains("\nName=Half-Life 2\n"));
        assert!(t.contains("\nExec=\"/opt/kettle app/kettle\" --play 220\n"));
        assert!(t.contains("\nCategories=Game;\n"));
        assert!(t.ends_with('\n'));
    }

    #[test]
    fn names_cannot_inject_lines() {
        let t = desktop_entry("Evil\nExec=rm -rf ~", 1, "/bin/kettle", "kettle");
        assert_eq!(t.matches("\nExec=").count(), 1);
    }

    #[test]
    fn exec_quoting_escapes_specials() {
        assert_eq!(exec_quote("/a b/$x\"y%"), "\"/a b/\\$x\\\"y%%\"");
    }

    #[test]
    fn finds_the_desktop_folder() {
        let home = Path::new("/home/u");
        let txt = "XDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n";
        assert_eq!(parse_desktop_dir(txt, home), Some(PathBuf::from("/home/u/Schreibtisch")));
        assert_eq!(parse_desktop_dir("", home), None);
        // "$HOME" alone means "no desktop folder": ignored.
        assert_eq!(parse_desktop_dir("XDG_DESKTOP_DIR=\"$HOME\"", home), None);
    }
}
