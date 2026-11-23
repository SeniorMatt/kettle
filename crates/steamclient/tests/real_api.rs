//! Loads Valve's own redistributable `libsteam_api.so` (the library every native Steamworks game ships) against
//! Kettle's `libsteamclient.so` and a fake host, and calls `SteamAPI_Init`. This is the closest check to a real
//! game that needs no game: the real library walks our `CreateInterface`, `ISteamClient` and the version strings it
//! was built with, and fails if a table is wrong.
//!
//! The library is looked for read-only in the user's own Steam install and library folders (`~/.steam/steam`,
//! `~/.local/share/Steam`, the Flatpak path, and `KETTLE_LIBSTEAM_API=<file>`); it is copied into a temp folder
//! before it is loaded. Without one the test prints a note and passes. Steam itself is never started: the library
//! is pointed at a temporary home whose `.steam/sdk64/steamclient.so` is our library.

use libloading::{Library, Symbol};
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::io::{BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use steamclient::proto::*;

const ME: u64 = 76561197960265729 + 1000;
const APP: u32 = 480;

fn answer(req: Request) -> Reply {
    match req {
        Request::Hello { .. } => Reply::Hello(Identity {
            steam_id: ME,
            persona: "Tester".into(),
            logged_on: true,
            language: "english".into(),
        }),
        Request::Owns { app } => Reply::Owns { owned: app == APP },
        Request::GetStats { .. } => Reply::Stats { eresult: 1, stats: BTreeMap::new(), achievements: BTreeMap::new() },
        Request::StoreStats { .. } => Reply::Stored { eresult: 1 },
        Request::AppInfo { .. } => Reply::AppInfo(AppInfo { install_dir: Some("/games/x".into()), build_id: 1 }),
        _ => Reply::Error { message: "unused".into() },
    }
}

fn serve(listener: UnixListener) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            std::thread::spawn(move || {
                let mut w = stream.try_clone().unwrap();
                let mut r = BufReader::new(stream);
                while let Ok(Some(line)) = read_line_limited(&mut r) {
                    let req: RequestLine = serde_json::from_str(&line).unwrap();
                    let mut out = serde_json::to_string(&ReplyLine { id: req.id, rep: answer(req.req) }).unwrap();
                    out.push('\n');
                    if w.write_all(out.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
}

fn lib_path() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .flat_map(|d| [d.join("libsteamclient.so"), d.join("deps").join("libsteamclient.so")])
        .find(|p| p.exists())
        .unwrap_or_else(|| panic!("build the cdylib first (cargo build -p sr-steamclient); test at {}", exe.display()))
}

/// A 64-bit ELF shared object?
fn is_elf64(p: &Path) -> bool {
    std::fs::read(p).is_ok_and(|b| b.len() > 64 && b[..4] == [0x7f, b'E', b'L', b'F'] && b[4] == 2)
}

/// Look for `libsteam_api.so` below `dir`, a few levels deep, never following links.
fn find_below(dir: &Path, depth: usize) -> Option<PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let path = e.path();
        if t.is_file() && e.file_name() == "libsteam_api.so" && is_elf64(&path) {
            return Some(path);
        }
        if t.is_dir()
            && depth > 0
            && let Some(p) = find_below(&path, depth - 1)
        {
            return Some(p);
        }
    }
    None
}

fn find_libsteam_api() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KETTLE_LIBSTEAM_API").map(PathBuf::from).filter(|p| is_elf64(p)) {
        return Some(p);
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    let roots = [
        home.join(".steam/steam/steamapps/common"),
        home.join(".local/share/Steam/steamapps/common"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/common"),
    ];
    for root in roots {
        let Ok(games) = std::fs::read_dir(&root) else { continue };
        for g in games.flatten() {
            let name = g.file_name().to_string_lossy().into_owned();
            if name.starts_with("Proton") || name.starts_with("Steam Linux Runtime") {
                continue; // those hold Valve's own copies for other purposes
            }
            if let Some(p) = find_below(&g.path(), 3) {
                return Some(p);
            }
        }
    }
    None
}

#[test]
fn valves_libsteam_api_initialises_against_our_steamclient() {
    let Some(found) = find_libsteam_api() else {
        eprintln!("skipped: no libsteam_api.so found in the Steam install (set KETTLE_LIBSTEAM_API to a file)");
        return;
    };
    let tmp = std::env::temp_dir().join(format!("kettle-real-api-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let home = tmp.join("home");
    std::fs::create_dir_all(home.join(".steam/sdk64")).unwrap();
    let api = tmp.join("libsteam_api.so");
    std::fs::copy(&found, &api).unwrap(); // the original stays untouched
    let ours = lib_path();
    std::os::unix::fs::symlink(&ours, home.join(".steam/sdk64/steamclient.so")).unwrap();
    // What the library reads to decide "Steam is running": our own pid, so no real client is involved.
    let pid = std::process::id();
    std::fs::write(home.join(".steam/steam.pid"), pid.to_string()).unwrap();
    std::fs::write(
        home.join(".steam/registry.vdf"),
        format!(
            "\"Registry\"\n{{\n\t\"HKCU\"\n\t{{\n\t\t\"Software\"\n\t\t{{\n\t\t\t\"Valve\"\n\t\t\t{{\n\t\t\t\t\"Steam\"\n\t\t\t\t{{\n\t\t\t\t\t\"ActiveProcess\"\n\t\t\t\t\t{{\n\t\t\t\t\t\t\"pid\"\t\t\"{pid}\"\n\t\t\t\t\t\t\"SteamClientDll64\"\t\t\"{}\"\n\t\t\t\t\t\t\"ActiveUser\"\t\t\"1000\"\n\t\t\t\t\t}}\n\t\t\t\t}}\n\t\t\t}}\n\t\t}}\n\t}}\n}}\n",
            ours.display()
        ),
    )
    .unwrap();
    let sock = tmp.join("host.sock");
    serve(UnixListener::bind(&sock).unwrap());
    std::fs::write(tmp.join("steam_appid.txt"), APP.to_string()).unwrap();
    // SAFETY: the only test of this file; nothing else reads the environment concurrently.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("KETTLE_STEAMCLIENT_SOCK", &sock);
        std::env::set_var("SteamAppId", APP.to_string());
        std::env::set_var("SteamGameId", APP.to_string());
    }
    std::env::set_current_dir(&tmp).unwrap();

    // SAFETY: Valve's library, called with its documented C signatures.
    unsafe {
        let lib = Library::new(&api).unwrap_or_else(|e| panic!("dlopen {}: {e}", api.display()));
        let init: Symbol<unsafe extern "C" fn() -> bool> = lib.get(b"SteamAPI_Init").expect("SteamAPI_Init");
        let ok = init();
        assert!(ok, "SteamAPI_Init failed against Kettle's steamclient (library: {})", found.display());
        let hsteamuser: Symbol<unsafe extern "C" fn() -> i32> = lib.get(b"SteamAPI_GetHSteamUser").expect("user");
        assert_ne!(hsteamuser(), 0);
        let run: Symbol<unsafe extern "C" fn()> = lib.get(b"SteamAPI_RunCallbacks").expect("RunCallbacks");
        run();
        let shutdown: Symbol<unsafe extern "C" fn()> = lib.get(b"SteamAPI_Shutdown").expect("Shutdown");
        shutdown();
        let _: *const c_void = std::ptr::null();
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
