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
/// How long we wait for a sign of the client's IPC after its process shows up, before we go on anyway (a client
/// whose signals we cannot read, such as Flatpak's, must not block a game forever).
const IPC_GRACE: Duration = Duration::from_secs(8);
/// Flatpak's client hides its IPC from us: it counts as ready this long after its process started.
const FLATPAK_WARMUP: Duration = Duration::from_secs(4);
const POLL: Duration = Duration::from_millis(100);

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
        is_running_with(&pid_files(), |pid| proc_name(pid).is_some_and(|n| looks_like_steam(&n))) || any_steam_process()
    }
}

/// Look for a running `steam` process of ours in /proc. Needed for Flatpak
/// Steam: its pid file holds a pid from inside the sandbox, which means
/// nothing on the host, but the process itself is visible here. Only runs
/// when the player presses Play, so the scan's cost doesn't matter.
#[cfg(target_os = "linux")]
fn any_steam_pid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let me = std::fs::metadata("/proc/self").ok()?;
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|e| {
        let pid = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok())?;
        (e.metadata().is_ok_and(|m| m.uid() == me.uid()) && proc_name(pid).is_some_and(|n| looks_like_steam(&n)))
            .then_some(pid)
    })
}

#[cfg(target_os = "linux")]
fn any_steam_process() -> bool {
    any_steam_pid().is_some()
}

#[cfg(all(unix, not(target_os = "linux")))]
fn any_steam_process() -> bool {
    false
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

// ---- Is its IPC ready? ---------------------------------------------------------------------------------------

/// Pure rule: the client's IPC is up when its process is alive and it has announced itself, either in
/// `registry.vdf` (`ActiveProcess/pid` equals the process) or with its shared-memory object, which must be newer
/// than the process (an old file from a crashed run proves nothing).
pub fn ipc_ready_rule(pid: Option<u32>, registry_pid: Option<u32>, shm_fresh: bool) -> bool {
    pid.is_some() && (registry_pid == pid || shm_fresh)
}

/// `ActiveProcess { pid }` from the text of Steam's `registry.vdf`.
pub fn parse_active_pid(text: &str) -> Option<u32> {
    let at = text.find("\"ActiveProcess\"")?;
    let rest = &text[at..];
    let end = rest.find('}').unwrap_or(rest.len());
    let block = &rest[..end];
    let p = block.find("\"pid\"")?;
    let after = &block[p + 5..];
    let q1 = after.find('"')?;
    let after = &after[q1 + 1..];
    let q2 = after.find('"')?;
    after[..q2].trim().parse().ok().filter(|p| *p > 0)
}

/// The pid of the live client process, from the pid files (Windows: the registry).
#[cfg(not(windows))]
fn live_pid() -> Option<u32> {
    pid_files().iter().find_map(|f| {
        let pid = std::fs::read_to_string(f).ok()?.trim().parse::<u32>().ok()?;
        (pid > 0 && proc_name(pid).is_some_and(|n| looks_like_steam(&n))).then_some(pid)
    })
}

/// When the process `pid` started, in unix seconds (from `/proc`). `None` if unknown.
#[cfg(target_os = "linux")]
fn process_start(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after the `)` that closes the command name; starttime is field 22 overall = index 19 after it.
    let ticks: u64 = stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()?;
    let boot: u64 = std::fs::read_to_string("/proc/stat")
        .ok()?
        .lines()
        .find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())?;
    Some(boot + ticks / 100) // CLK_TCK is 100 on every Linux we run on
}

/// The client creates `/dev/shm/u<uid>-ValveIPCSharedObj-Steam` once it serves IPC. Fresh = newer than `pid`.
#[cfg(target_os = "linux")]
fn shm_fresh(pid: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(me) = std::fs::metadata("/proc/self") else { return false };
    let Ok(m) = std::fs::metadata(format!("/dev/shm/u{}-ValveIPCSharedObj-Steam", me.uid())) else { return false };
    let modified = m.mtime().max(0) as u64;
    process_start(pid).is_some_and(|start| modified + 1 >= start)
}

/// Is the client up and answering? Linux: see [`ipc_ready_rule`]. Windows: the process runs.
pub fn ipc_ready() -> bool {
    #[cfg(windows)]
    {
        is_running()
    }
    #[cfg(not(windows))]
    {
        let Some(pid) = live_pid() else {
            // Flatpak: its pid file holds a pid from inside the sandbox and its /dev/shm is private. All we can
            // see is the process; give it a few seconds from its start.
            #[cfg(target_os = "linux")]
            if let Some(p) = any_steam_pid() {
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                return process_start(p).is_some_and(|st| now >= st + FLATPAK_WARMUP.as_secs());
            }
            return false;
        };
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        let registry =
            std::fs::read_to_string(home.join(".steam/registry.vdf")).ok().and_then(|t| parse_active_pid(&t));
        #[cfg(target_os = "linux")]
        let fresh = shm_fresh(pid);
        #[cfg(not(target_os = "linux"))]
        let fresh = false;
        ipc_ready_rule(Some(pid), registry, fresh)
    }
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

/// Wrap a start command so the client runs at low priority (`nice`, `ionice`) where those tools exist. Used for
/// the background start before the player asked for anything; a start the player waits for is not wrapped.
pub fn low_priority(spec: StartSpec, path: &[PathBuf]) -> StartSpec {
    // `nice -n 10 ionice -c 2 -n 7 <program> <args>`; with only one tool present the chain is shorter.
    let tools: [(&str, [&str; 4]); 2] = [("nice", ["-n", "10", "", ""]), ("ionice", ["-c", "2", "-n", "7"])];
    let mut first: Option<PathBuf> = None;
    let mut args: Vec<String> = Vec::new();
    for (tool, a) in tools {
        let Some(p) = find_in(path, tool) else { continue };
        if first.is_none() {
            first = Some(p);
        } else {
            args.push(tool.to_string());
        }
        args.extend(a.iter().filter(|x| !x.is_empty()).map(|x| x.to_string()));
    }
    let Some(program) = first else { return spec };
    args.push(spec.program.to_string_lossy().into_owned());
    args.extend(spec.args);
    StartSpec { program, args }
}

/// The command for the background start: [`start_spec`] at low priority.
pub fn background_command() -> Option<Command> {
    let path: Vec<PathBuf> = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
    start_spec().map(|s| {
        #[cfg(not(windows))]
        let s = low_priority(s, &path);
        #[cfg(windows)]
        let _ = &path;
        let mut c = Command::new(s.program);
        c.args(s.args);
        c
    })
}

/// Start the client now if it is not running, and return at once (the start-up of Kettle must not wait).
pub fn start_background() -> Result<Started> {
    if is_running() {
        return Ok(Started::AlreadyRunning);
    }
    spawn_client(background_command, &log_path())
}

/// Make sure the client runs and answers: start it if needed (at once, not in the background) and wait until
/// its IPC is ready, at most `timeout`. Returns immediately when it is already up.
pub fn ensure_running(timeout: Duration, cancel: &AtomicBool) -> Result<Started> {
    ensure_running_with(is_running, ipc_ready, start_command, &log_path(), timeout, IPC_GRACE, cancel)
}

fn spawn_client(command: impl FnOnce() -> Option<Command>, log: &Path) -> Result<Started> {
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
    Ok(Started::StartedNow)
}

/// [`ensure_running`] with everything injected, for tests. `grace` bounds the wait for `ready` once `running`.
pub fn ensure_running_with(
    running: impl Fn() -> bool,
    ready: impl Fn() -> bool,
    command: impl FnOnce() -> Option<Command>,
    log: &Path,
    timeout: Duration,
    grace: Duration,
    cancel: &AtomicBool,
) -> Result<Started> {
    let mut started = Started::AlreadyRunning;
    if !running() {
        if spawn_client(command, log)? == Started::NotInstalled {
            return Ok(Started::NotInstalled);
        }
        started = Started::StartedNow;
    }
    // Running (maybe started by the background start a moment ago): wait for the IPC, not for a fixed time.
    let begin = Instant::now();
    let mut running_since: Option<Instant> = None;
    loop {
        if running() {
            if ready() {
                return Ok(started);
            }
            let since = *running_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= grace {
                return Ok(started); // no signal we can read: go on, the game's own retries cover the rest
            }
        }
        if cancel.load(Ordering::Relaxed) {
            bail!("cancelled while waiting for the Steam client");
        }
        if begin.elapsed() >= timeout {
            bail!("the Steam client did not start within {} s", timeout.as_secs());
        }
        std::thread::sleep(POLL);
    }
}

// ---- Which installed games need it? (cached) ------------------------------------------------------------------

/// What we remember about one install: valid while the folder and the stamp (build id, update time) stay the same.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScanEntry {
    pub dir: PathBuf,
    pub stamp: u64,
    pub uses: bool,
}

/// The per-install answers of [`uses_steamworks`], so the start of Kettle does not walk every game folder again.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScanCache {
    pub apps: std::collections::BTreeMap<String, ScanEntry>,
}

impl ScanCache {
    pub fn path() -> PathBuf {
        store::data_dir().join("steamworks-scan.json")
    }

    pub fn load() -> ScanCache {
        store::load_json(&ScanCache::path()).unwrap_or_default()
    }

    pub fn save(&self) {
        let _ = store::save_json(&ScanCache::path(), self);
    }

    /// Does `app` (installed in `dir`, `stamp` = build id mixed with the update time) ship the Steamworks library?
    /// Walks the folder only when the cache has no matching entry. The bool says whether the cache changed.
    pub fn uses(&mut self, app: u32, dir: &Path, stamp: u64, scan: impl FnOnce(&Path) -> bool) -> (bool, bool) {
        if let Some(e) = self.apps.get(&app.to_string())
            && e.dir == dir
            && e.stamp == stamp
        {
            return (e.uses, false);
        }
        let uses = scan(dir);
        self.apps.insert(app.to_string(), ScanEntry { dir: dir.to_path_buf(), stamp, uses });
        (uses, true)
    }
}

/// The "Start Steam in the background" choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// Only when an installed game uses the Steamworks API.
    #[default]
    Automatic,
    Always,
    Never,
}

/// Should Kettle start the client in the background right now?
pub fn should_start_background(mode: Background, any_game_needs_it: bool, installed: bool, running: bool) -> bool {
    installed
        && !running
        && match mode {
            Background::Never => false,
            Background::Always => true,
            Background::Automatic => any_game_needs_it,
        }
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
        let readyfile = d.join("fake.ready");
        let script = d.join("fake-steam.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nsleep 1\necho $$ > '{}'\nsleep 1\ntouch '{}'\n",
                pidfile.display(),
                readyfile.display()
            ),
        )
        .unwrap();
        let mut c = Command::new("/bin/sh");
        c.arg(&script).arg("-silent");
        let cancel = AtomicBool::new(false);
        let running = || pidfile.exists();
        let ready = || readyfile.exists();
        let t = Instant::now();
        let r = ensure_running_with(
            running,
            ready,
            || Some(c),
            &d.join("log/steam.log"),
            Duration::from_secs(20),
            Duration::from_secs(10),
            &cancel,
        );
        assert_eq!(r.unwrap(), Started::StartedNow);
        // It waited for the ready signal (about 2 s), not for a fixed time and not for the grace.
        assert!(t.elapsed() >= Duration::from_secs(2) && t.elapsed() < Duration::from_secs(8), "{:?}", t.elapsed());
        // Already running and ready: the command is never asked for and there is no delay at all.
        let t = Instant::now();
        let r = ensure_running_with(
            running,
            ready,
            || panic!("not needed"),
            &d.join("l"),
            Duration::from_secs(1),
            IPC_GRACE,
            &cancel,
        );
        assert_eq!(r.unwrap(), Started::AlreadyRunning);
        assert!(t.elapsed() < Duration::from_millis(50), "no settle delay: {:?}", t.elapsed());
        // Running but no readable signal: goes on after the grace, not forever.
        let t = Instant::now();
        let r = ensure_running_with(
            running,
            || false,
            || panic!("not needed"),
            &d.join("l"),
            Duration::from_secs(5),
            Duration::from_millis(300),
            &cancel,
        );
        assert_eq!(r.unwrap(), Started::AlreadyRunning);
        assert!(t.elapsed() < Duration::from_secs(2));
        // not installed
        let r =
            ensure_running_with(|| false, || false, || None, &d.join("l"), Duration::from_secs(1), IPC_GRACE, &cancel);
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
        let r =
            ensure_running_with(|| false, || false, mk, &d.join("l"), Duration::from_millis(700), IPC_GRACE, &cancel);
        assert!(r.unwrap_err().to_string().contains("did not start"));
        let cancel = AtomicBool::new(true);
        let r = ensure_running_with(|| false, || false, mk, &d.join("l"), Duration::from_secs(5), IPC_GRACE, &cancel);
        assert!(r.unwrap_err().to_string().contains("cancelled"));
    }

    #[test]
    fn ipc_readiness_rule() {
        assert!(!ipc_ready_rule(None, Some(5), true), "no live process is never ready");
        assert!(!ipc_ready_rule(Some(5), None, false), "alive but silent: not ready (a stale pipe proves nothing)");
        assert!(!ipc_ready_rule(Some(5), Some(4), false), "registry names another (old) process");
        assert!(ipc_ready_rule(Some(5), Some(5), false), "registry announces this process");
        assert!(ipc_ready_rule(Some(5), None, true), "fresh shared-memory object");
    }

    #[test]
    fn active_process_pid_is_read_from_registry_vdf() {
        let text = "\"Registry\"\n{\n\t\"HKCU\"\n\t{\n\t\t\"Software\"\n\t\t{\n\t\t\t\"Valve\"\n\t\t\t{\n\t\t\t\t\"Steam\"\n\t\t\t\t{\n\t\t\t\t\t\"language\"\t\t\"english\"\n\t\t\t\t\t\"ActiveProcess\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"pid\"\t\t\"73430\"\n\t\t\t\t\t\t\"SteamClientDll64\"\t\t\"/x/steamclient.so\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";
        assert_eq!(parse_active_pid(text), Some(73430));
        assert_eq!(parse_active_pid("\"ActiveProcess\" { \"pid\" \"0\" }"), None);
        assert_eq!(parse_active_pid("\"Other\" { \"pid\" \"7\" }"), None);
        assert_eq!(parse_active_pid("\"ActiveProcess\" { \"x\" \"1\" } \"pid\" \"9\""), None, "pid of another block");
        assert_eq!(parse_active_pid(""), None);
    }

    #[test]
    fn automatic_background_decision() {
        use Background::*;
        // (mode, a game needs it, client installed, already running) -> start?
        assert!(should_start_background(Automatic, true, true, false));
        assert!(!should_start_background(Automatic, false, true, false), "no Steamworks game: stay out of the way");
        assert!(should_start_background(Always, false, true, false));
        assert!(!should_start_background(Never, true, true, false));
        assert!(!should_start_background(Always, true, false, false), "not installed");
        assert!(!should_start_background(Always, true, true, true), "never twice");
    }

    #[test]
    fn scan_cache_is_per_install_and_stamp() {
        let mut c = ScanCache::default();
        let dir = PathBuf::from("/games/a");
        let mut scans = 0;
        let mut ask = |c: &mut ScanCache, dir: &Path, stamp: u64, answer: bool| {
            c.uses(10, dir, stamp, |_| {
                scans += 1;
                answer
            })
        };
        assert_eq!(ask(&mut c, &dir, 1, true), (true, true), "first time: scanned");
        assert_eq!(ask(&mut c, &dir, 1, false), (true, false), "same install and build: answered from the cache");
        assert_eq!(ask(&mut c, &dir, 2, false), (false, true), "a new build is scanned again");
        assert_eq!(ask(&mut c, Path::new("/games/b"), 2, true), (true, true), "moved: scanned again");
        assert_eq!(scans, 3);
        let back: ScanCache = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn background_start_runs_at_low_priority() {
        let bin = tmp("prio");
        let spec = StartSpec { program: PathBuf::from("/usr/bin/steam"), args: vec!["-silent".into()] };
        assert_eq!(low_priority(spec.clone(), std::slice::from_ref(&bin)), spec, "no tools: unchanged");
        fs::write(bin.join("nice"), b"").unwrap();
        let l = low_priority(spec.clone(), std::slice::from_ref(&bin));
        assert_eq!(l.program, bin.join("nice"));
        assert_eq!(l.args, ["-n", "10", "/usr/bin/steam", "-silent"]);
        fs::write(bin.join("ionice"), b"").unwrap();
        let l = low_priority(spec, std::slice::from_ref(&bin));
        assert_eq!(l.args, ["-n", "10", "ionice", "-c", "2", "-n", "7", "/usr/bin/steam", "-silent"]);
    }
}
