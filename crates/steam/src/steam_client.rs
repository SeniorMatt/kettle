//! The official Steam client as a helper for games that use the Steamworks API.
//!
//! Such games call `SteamAPI_Init`, which only succeeds while the official client runs. We never talk to it; we
//! only notice whether it runs and, if the user allows it, start it minimized (`-silent`) before the game.
//!
//! Pure helpers take their paths as arguments so tests can use temp dirs; the public wrappers add the real ones.

use crate::store;
use anyhow::{Context, Result, bail};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Names of the Steamworks library, lower case.
const API_LIBS: [&str; 3] = ["libsteam_api.so", "steam_api.dll", "steam_api64.dll"];
const MAX_DEPTH: usize = 4;
const MAX_ENTRIES: usize = 5000;
/// Time Steam gets to finish starting its IPC after the pid file shows up.
const SETTLE: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(500);

/// Result of [`ensure_running`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    AlreadyRunning,
    StartedNow,
    NotInstalled,
}

// ---- Does the game need it? ----------------------------------------------------------------------------------

/// Does the install folder contain a Steamworks library? Bounded walk, symlinks are not followed.
pub fn uses_steamworks(install_dir: &Path) -> bool {
    let mut queue = VecDeque::from([(install_dir.to_path_buf(), 0usize)]);
    let mut seen = 0;
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                return false;
            }
            let Ok(ft) = entry.file_type() else { continue }; // does not follow symlinks
            if ft.is_dir() {
                if depth < MAX_DEPTH {
                    queue.push_back((entry.path(), depth + 1));
                }
            } else if ft.is_file() && API_LIBS.contains(&entry.file_name().to_string_lossy().to_lowercase().as_str()) {
                return true;
            }
        }
    }
    false
}

// ---- Is it running? ------------------------------------------------------------------------------------------

/// Does a process name (`/proc/<pid>/comm` or an exe file name) look like the Steam client?
fn looks_like_steam(name: &str) -> bool {
    let n = name.trim().to_lowercase();
    n == "steam" || n == "steam.exe"
}

/// Pid files of the official client on Linux: native and Flatpak.
#[cfg(not(windows))]
fn pid_files() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return vec![] };
    vec![home.join(".steam/steam.pid"), home.join(".var/app/com.valvesoftware.Steam/.steam/steam.pid")]
}

#[cfg(not(windows))]
fn proc_name(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok();
    comm.or_else(|| {
        let exe = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        Some(exe.file_name()?.to_string_lossy().into_owned())
    })
}

/// Is the client running? Linux: a pid file with a live Steam process. Windows: the registry's `ActiveProcess`.
pub fn is_running() -> bool {
    #[cfg(windows)]
    {
        windows_active_pid().is_some_and(windows_pid_alive)
    }
    #[cfg(not(windows))]
    {
        is_running_with(&pid_files(), |pid| proc_name(pid).is_some_and(|n| looks_like_steam(&n)))
    }
}

/// True when one of `pid_files` holds a pid that `is_steam` accepts. `is_steam` must be false for dead pids.
pub fn is_running_with(pid_files: &[PathBuf], is_steam: impl Fn(u32) -> bool) -> bool {
    pid_files.iter().any(|f| {
        std::fs::read_to_string(f).ok().and_then(|t| t.trim().parse::<u32>().ok()).is_some_and(|p| p > 0 && is_steam(p))
    })
}

#[cfg(windows)]
fn windows_active_pid() -> Option<u32> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam\\ActiveProcess").ok()?;
    key.get_value::<u32, _>("pid").ok().filter(|p| *p != 0)
}

#[cfg(windows)]
fn windows_pid_alive(pid: u32) -> bool {
    use std::os::windows::process::CommandExt;
    /// CREATE_NO_WINDOW: we are a GUI app, so a console would flash on screen.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    out.is_ok_and(|o| looks_like_steam_row(&String::from_utf8_lossy(&o.stdout)))
}

#[cfg(windows)]
fn looks_like_steam_row(text: &str) -> bool {
    text.split(',').next().is_some_and(|name| looks_like_steam(name.trim().trim_matches('"')))
}

// ---- Starting it ---------------------------------------------------------------------------------------------

/// A program and its arguments; turned into a `Command` by [`start_command`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
}

fn find_in(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Pick how to start the client on Linux. `path` are the PATH folders, `home` the home folder, `flatpak_dirs`
/// where a system-wide Flatpak install of Steam would be.
pub fn resolve_linux(
    path: &[PathBuf],
    home: &Path,
    flatpak_dirs: &[PathBuf],
    usr_bin_steam: &Path,
) -> Option<StartSpec> {
    let silent = || vec!["-silent".to_string()];
    if let Some(p) = find_in(path, "steam") {
        return Some(StartSpec { program: p, args: silent() });
    }
    if usr_bin_steam.is_file() {
        return Some(StartSpec { program: usr_bin_steam.to_path_buf(), args: silent() });
    }
    let has_flatpak_app =
        home.join(".var/app/com.valvesoftware.Steam").is_dir() || flatpak_dirs.iter().any(|d| d.is_dir());
    if has_flatpak_app && let Some(fp) = find_in(path, "flatpak") {
        let mut args = vec!["run".to_string(), "com.valvesoftware.Steam".to_string()];
        args.extend(silent());
        return Some(StartSpec { program: fp, args });
    }
    let snap = PathBuf::from("/snap/bin/steam");
    snap.is_file().then(|| StartSpec { program: snap, args: silent() })
}

/// How to start the installed client, `None` when there is none.
pub fn start_spec() -> Option<StartSpec> {
    #[cfg(windows)]
    {
        let root = crate::local::steam_roots().into_iter().next()?;
        let exe = root.join("steam.exe");
        exe.is_file().then(|| StartSpec { program: exe, args: vec!["-silent".into()] })
    }
    #[cfg(not(windows))]
    {
        let home = PathBuf::from(std::env::var_os("HOME")?);
        let path: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
        let flatpak = [PathBuf::from("/var/lib/flatpak/app/com.valvesoftware.Steam")];
        resolve_linux(&path, &home, &flatpak, Path::new("/usr/bin/steam"))
    }
}

/// The command that starts the client minimized, or `None` when it is not installed.
pub fn start_command() -> Option<Command> {
    start_spec().map(|s| {
        let mut c = Command::new(s.program);
        c.args(s.args);
        c
    })
}

/// Log of the client we started: our data dir, never inherited from the UI's terminal.
pub fn log_path() -> PathBuf {
    store::data_dir().join("logs").join("steam-client.log")
}

/// Make sure the client runs: start it if needed and wait (polling every 500 ms, at most `timeout`) until it does.
pub fn ensure_running(timeout: Duration, cancel: &AtomicBool) -> Result<Started> {
    ensure_running_with(is_running, start_command, &log_path(), timeout, SETTLE, cancel)
}

/// [`ensure_running`] with everything injected, for tests.
pub fn ensure_running_with(
    running: impl Fn() -> bool,
    command: impl FnOnce() -> Option<Command>,
    log: &Path,
    timeout: Duration,
    settle: Duration,
    cancel: &AtomicBool,
) -> Result<Started> {
    if running() {
        return Ok(Started::AlreadyRunning);
    }
    let Some(mut cmd) = command() else { return Ok(Started::NotInstalled) };
    if let Some(dir) = log.parent() {
        store::private_dir(dir)?;
    }
    let out = std::fs::File::create(log).with_context(|| format!("creating {}", log.display()))?;
    cmd.stdin(std::process::Stdio::null()).stdout(out.try_clone()?).stderr(out);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0); // detached from our group and Ctrl-C
    let mut child = cmd.spawn().context("starting the Steam client")?;
    // Reap the launcher script when it exits; the client itself keeps running on its own.
    std::thread::Builder::new().name("steam-client-wait".into()).spawn(move || {
        let _ = child.wait();
    })?;
    let start = Instant::now();
    while !running() {
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled while waiting for the Steam client");
        }
        if start.elapsed() >= timeout {
            bail!("the Steam client did not start within {} s", timeout.as_secs());
        }
        std::thread::sleep(POLL);
    }
    std::thread::sleep(settle);
    Ok(Started::StartedNow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sr-steamclient-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn finds_steamworks_libs_case_insensitively() {
        let d = tmp("libs");
        assert!(!uses_steamworks(&d));
        fs::create_dir_all(d.join("bin/x64")).unwrap();
        fs::write(d.join("bin/x64/Steam_API64.DLL"), b"").unwrap();
        assert!(uses_steamworks(&d));
        let d2 = tmp("libs2");
        fs::write(d2.join("libsteam_api.so"), b"").unwrap();
        assert!(uses_steamworks(&d2));
        assert!(!uses_steamworks(&d2.join("missing")));
    }

    #[test]
    fn walk_is_bounded() {
        let d = tmp("deep");
        let deep = d.join("a/b/c/d/e");
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("libsteam_api.so"), b"").unwrap();
        assert!(!uses_steamworks(&d), "depth 5 is too deep");
        fs::write(d.join("a/b/c/d/steam_api.dll"), b"").unwrap();
        assert!(uses_steamworks(&d), "depth 4 is found");
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_not_followed() {
        let d = tmp("links");
        let outside = tmp("links-out");
        fs::write(outside.join("libsteam_api.so"), b"").unwrap();
        std::os::unix::fs::symlink(&outside, d.join("link")).unwrap();
        assert!(!uses_steamworks(&d));
    }

    #[test]
    fn entry_budget_stops_the_walk() {
        let d = tmp("many");
        for i in 0..(MAX_ENTRIES + 10) {
            fs::write(d.join(format!("f{i:05}")), b"").unwrap();
        }
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub/libsteam_api.so"), b"").unwrap();
        assert!(!uses_steamworks(&d));
    }

    #[test]
    fn process_names() {
        assert!(looks_like_steam("steam\n"));
        assert!(looks_like_steam("Steam.exe"));
        assert!(!looks_like_steam("bash"));
    }

    #[test]
    fn pid_file_checks() {
        let d = tmp("pid");
        let me = std::process::id();
        let f = d.join("steam.pid");
        fs::write(&f, format!("{me}\n")).unwrap();
        assert!(is_running_with(std::slice::from_ref(&f), |p| p == me));
        assert!(!is_running_with(std::slice::from_ref(&f), |_| false), "name does not look like steam");
        assert!(!is_running_with(&[d.join("missing.pid")], |_| true));
        fs::write(&f, "garbage").unwrap();
        assert!(!is_running_with(std::slice::from_ref(&f), |_| true));
        fs::write(&f, "0").unwrap();
        assert!(!is_running_with(std::slice::from_ref(&f), |_| true));
    }

    #[cfg(unix)]
    #[test]
    fn dead_pid_is_not_running() {
        let d = tmp("dead");
        let mut child = Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let f = d.join("steam.pid");
        fs::write(&f, pid.to_string()).unwrap();
        let alive = |p: u32| Path::new(&format!("/proc/{p}")).exists();
        assert!(!is_running_with(&[f], alive));
    }

    #[cfg(unix)]
    #[test]
    fn linux_start_choice() {
        let home = tmp("home");
        let bin = tmp("bin");
        let none = home.join("no-usr-bin-steam");
        assert_eq!(resolve_linux(std::slice::from_ref(&bin), &home, &[], &none), None);
        // Flatpak only
        fs::write(bin.join("flatpak"), b"").unwrap();
        assert_eq!(resolve_linux(std::slice::from_ref(&bin), &home, &[], &none), None, "flatpak without the app");
        fs::create_dir_all(home.join(".var/app/com.valvesoftware.Steam")).unwrap();
        let spec = resolve_linux(std::slice::from_ref(&bin), &home, &[], &none).unwrap();
        assert_eq!(spec.args, ["run", "com.valvesoftware.Steam", "-silent"]);
        // Native wins
        fs::write(bin.join("steam"), b"").unwrap();
        let spec = resolve_linux(std::slice::from_ref(&bin), &home, &[], &none).unwrap();
        assert_eq!((spec.program, spec.args), (bin.join("steam"), vec!["-silent".to_string()]));
        // /usr/bin/steam fallback
        let usr = home.join("usr-bin-steam");
        fs::write(&usr, b"").unwrap();
        let empty = tmp("emptybin");
        assert_eq!(resolve_linux(&[empty], &home, &[], &usr).unwrap().program, usr);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_running_with_fake_command() {
        let d = tmp("ensure");
        let pidfile = d.join("fake.pid");
        let script = d.join("fake-steam.sh");
        fs::write(&script, format!("#!/bin/sh\nsleep 1\necho $$ > '{}'\n", pidfile.display())).unwrap();
        let mut c = Command::new("/bin/sh");
        c.arg(&script).arg("-silent");
        let cancel = AtomicBool::new(false);
        let running = || pidfile.exists();
        let t = Instant::now();
        let r = ensure_running_with(
            running,
            || Some(c),
            &d.join("log/steam.log"),
            Duration::from_secs(20),
            Duration::from_millis(100),
            &cancel,
        );
        assert_eq!(r.unwrap(), Started::StartedNow);
        assert!(t.elapsed() >= Duration::from_secs(1));
        // already running: the command is never asked for
        let r = ensure_running_with(
            running,
            || panic!("not needed"),
            &d.join("l"),
            Duration::from_secs(1),
            SETTLE,
            &cancel,
        );
        assert_eq!(r.unwrap(), Started::AlreadyRunning);
        // not installed
        let r = ensure_running_with(|| false, || None, &d.join("l"), Duration::from_secs(1), SETTLE, &cancel);
        assert_eq!(r.unwrap(), Started::NotInstalled);
    }

    #[cfg(unix)]
    #[test]
    fn ensure_running_times_out_and_cancels() {
        let d = tmp("timeout");
        let mk = || {
            let mut c = Command::new("/bin/sh");
            c.args(["-c", "exit 0"]);
            Some(c)
        };
        let cancel = AtomicBool::new(false);
        let r = ensure_running_with(|| false, mk, &d.join("l"), Duration::from_millis(700), SETTLE, &cancel);
        assert!(r.unwrap_err().to_string().contains("did not start"));
        let cancel = AtomicBool::new(true);
        let r = ensure_running_with(|| false, mk, &d.join("l"), Duration::from_secs(5), SETTLE, &cancel);
        assert!(r.unwrap_err().to_string().contains("cancelled"));
    }
}
