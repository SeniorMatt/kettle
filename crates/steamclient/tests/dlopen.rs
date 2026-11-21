//! Loads the built `libsteamclient.so` the way `libsteam_api.so` and Proton do (dlopen + dlsym, then
//! `CreateInterface("SteamClient021")` and calls through the vtables) against a fake Kettle on a Unix socket.
//! No Valve binaries are involved. One test function: the library's state is process-wide.

use libloading::{Library, Symbol};
use std::collections::BTreeMap;
use std::ffi::{CString, c_char, c_void};
use std::io::{BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use steamclient::proto::*;

const ME: u64 = 76561197960265729 + 1000;
const APP: u32 = 480;

#[derive(Default)]
struct Fake {
    stats: BTreeMap<String, StatValue>,
    achievements: BTreeMap<String, u32>,
    stores: u32,
    hello_app: u32,
}

fn serve(listener: UnixListener, fake: Arc<Mutex<Fake>>) {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let fake = fake.clone();
            std::thread::spawn(move || {
                let mut w = stream.try_clone().unwrap();
                let mut r = BufReader::new(stream);
                while let Ok(Some(line)) = read_line_limited(&mut r) {
                    let req: RequestLine = serde_json::from_str(&line).unwrap();
                    let rep = answer(&fake, req.req);
                    let mut out = serde_json::to_string(&ReplyLine { id: req.id, rep }).unwrap();
                    out.push('\n');
                    if w.write_all(out.as_bytes()).is_err() {
                        return;
                    }
                }
            });
        }
    });
}

fn answer(fake: &Mutex<Fake>, req: Request) -> Reply {
    let mut f = fake.lock().unwrap();
    match req {
        Request::Hello { app } => {
            f.hello_app = app;
            Reply::Hello(Identity {
                steam_id: ME,
                persona: "Tester".into(),
                logged_on: true,
                language: "german".into(),
            })
        }
        Request::Owns { app } => Reply::Owns { owned: app == APP },
        Request::EncryptedTicket { userdata, .. } => {
            assert_eq!(hex_decode(&userdata).unwrap(), b"hi");
            Reply::Ticket { eresult: 1, ticket: hex_encode(&[1, 2, 3]) }
        }
        Request::AuthTicket { .. } => Reply::Ticket { eresult: 1, ticket: hex_encode(&[9, 8, 7, 6]) },
        Request::AppTicket { .. } => Reply::Error { message: "unused".into() },
        Request::GetStats { .. } => {
            Reply::Stats { eresult: 1, stats: f.stats.clone(), achievements: f.achievements.clone() }
        }
        Request::StoreStats { stats, achievements, .. } => {
            f.stats = stats;
            f.achievements = achievements;
            f.stores += 1;
            Reply::Stored { eresult: 1 }
        }
        Request::AppInfo { .. } => {
            Reply::AppInfo(AppInfo { install_dir: Some("/games/spacewar".into()), build_id: 1234 })
        }
    }
}

fn lib_path() -> PathBuf {
    // The test binary sits somewhere below target/<profile> (deps/ or build/<crate>/<hash>/, depending on the
    // cargo version); the cdylib is target/<profile>/libsteamclient.so.
    let exe = std::env::current_exe().unwrap();
    let found = exe
        .ancestors()
        .flat_map(|d| [d.join("libsteamclient.so"), d.join("deps").join("libsteamclient.so")])
        .find(|p| p.exists());
    found.unwrap_or_else(|| panic!("build the cdylib first (cargo build -p sr-steamclient); test at {}", exe.display()))
}

type This = *mut c_void;
type Get3 = unsafe extern "C" fn(This, i32, i32, *const c_char) -> This;
type Get2 = unsafe extern "C" fn(This, i32, *const c_char) -> This;

/// Slot `i` of the interface object `obj`.
unsafe fn slot<F: Copy>(obj: This, i: usize) -> F {
    assert!(!obj.is_null());
    // SAFETY: an interface object is a pointer to a table of function pointers.
    unsafe {
        let vt = *(obj as *const *const usize);
        std::mem::transmute_copy(&*vt.add(i))
    }
}

#[repr(C)]
struct CallbackMsg {
    user: i32,
    callback: i32,
    param: *mut u8,
    size: i32,
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

#[test]
fn games_can_use_kettles_steamclient() {
    let sock = std::env::temp_dir().join(format!("kettle-sc-test-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&sock);
    let fake = Arc::new(Mutex::new(Fake::default()));
    {
        let mut f = fake.lock().unwrap();
        f.stats.insert("kills".into(), StatValue::Int(7));
        f.achievements.insert("ACH_OLD".into(), 1_700_000_000);
    }
    serve(UnixListener::bind(&sock).unwrap(), fake.clone());
    // SAFETY: single test in this process, nothing else reads the environment concurrently.
    unsafe {
        std::env::set_var("KETTLE_STEAMCLIENT_SOCK", &sock);
        std::env::set_var("SteamAppId", APP.to_string());
    }

    // SAFETY: loading our own library; every symbol is used with its real signature.
    unsafe {
        let lib = Library::new(lib_path()).expect("dlopen");
        // Proton refuses to start without these.
        for name in [
            "CreateInterface",
            "Steam_BGetCallback",
            "Steam_GetAPICallResult",
            "Steam_FreeLastCallback",
            "Steam_ReleaseThreadLocalMemory",
            "Steam_IsKnownInterface",
            "Steam_NotifyMissingInterface",
        ] {
            lib.get::<*const c_void>(name.as_bytes()).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        let create: Symbol<unsafe extern "C" fn(*const c_char, *mut i32) -> This> =
            lib.get(b"CreateInterface").unwrap();
        let bget: Symbol<unsafe extern "C" fn(i32, *mut CallbackMsg, *mut i32) -> bool> =
            lib.get(b"Steam_BGetCallback").unwrap();
        let free_last: Symbol<unsafe extern "C" fn(i32) -> bool> = lib.get(b"Steam_FreeLastCallback").unwrap();
        let api_result: Symbol<unsafe extern "C" fn(i32, u64, *mut u8, i32, i32, *mut bool) -> bool> =
            lib.get(b"Steam_GetAPICallResult").unwrap();
        let known: Symbol<unsafe extern "C" fn(*const c_char) -> bool> = lib.get(b"Steam_IsKnownInterface").unwrap();
        let drain = |pipe: i32| -> Vec<(i32, Vec<u8>)> {
            let mut out = Vec::new();
            let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
            let mut call = -1;
            while bget(pipe, &mut msg, &mut call) {
                out.push((msg.callback, std::slice::from_raw_parts(msg.param, msg.size as usize).to_vec()));
                assert!(free_last(pipe));
            }
            out
        };

        // Unknown names: no interface, a non-zero return code.
        let mut rc = -1;
        assert!(create(c("Bogus").as_ptr(), &mut rc).is_null());
        assert_ne!(rc, 0);
        assert!(known(c("SteamClient021").as_ptr()));
        assert!(!known(c("Bogus").as_ptr()));

        let client = create(c("SteamClient021").as_ptr(), &mut rc);
        assert_eq!(rc, 0);
        let create_pipe: unsafe extern "C" fn(This) -> i32 = slot(client, 0);
        let connect: unsafe extern "C" fn(This, i32) -> i32 = slot(client, 2);
        let pipe = create_pipe(client);
        assert!(pipe > 0);
        let user = connect(client, pipe);
        assert_eq!(user, 1);
        let get3 = |idx: usize, ver: &str| -> This {
            let f: Get3 = slot(client, idx);
            f(client, user, pipe, c(ver).as_ptr())
        };

        // ISteamUser
        let su = get3(5, "SteamUser021");
        let logged_on: unsafe extern "C" fn(This) -> bool = slot(su, 1);
        let steam_id: unsafe extern "C" fn(This) -> u64 = slot(su, 2);
        assert!(logged_on(su));
        assert_eq!(steam_id(su), ME);
        assert_eq!(fake.lock().unwrap().hello_app, APP);
        assert_eq!(get3(5, "SteamUser021"), su, "same object every time");

        // ISteamApps
        let apps = get3(15, "STEAMAPPS_INTERFACE_VERSION008");
        let subscribed: unsafe extern "C" fn(This) -> bool = slot(apps, 0);
        let subscribed_app: unsafe extern "C" fn(This, u32) -> bool = slot(apps, 6);
        let language: unsafe extern "C" fn(This) -> *const c_char = slot(apps, 4);
        let install_dir: unsafe extern "C" fn(This, u32, *mut c_char, u32) -> u32 = slot(apps, 18);
        let build_id: unsafe extern "C" fn(This) -> i32 = slot(apps, 23);
        assert!(subscribed(apps));
        assert!(subscribed_app(apps, 480));
        assert!(!subscribed_app(apps, 440), "an app the account does not own is never reported as owned");
        assert_eq!(std::ffi::CStr::from_ptr(language(apps)).to_str().unwrap(), "german");
        let mut buf = [0 as c_char; 64];
        let n = install_dir(apps, 480, buf.as_mut_ptr(), 64);
        assert_eq!(std::ffi::CStr::from_ptr(buf.as_ptr()).to_str().unwrap(), "/games/spacewar");
        assert_eq!(n, 16);
        assert_eq!(build_id(apps), 1234);

        // ISteamUtils takes (pipe, version) only.
        let get_utils: Get2 = slot(client, 9);
        let utils = get_utils(client, pipe, c("SteamUtils010").as_ptr());
        let app_id: unsafe extern "C" fn(This) -> u32 = slot(utils, 9);
        let overlay: unsafe extern "C" fn(This) -> bool = slot(utils, 17);
        assert_eq!(app_id(utils), APP);
        assert!(!overlay(utils));

        // ISteamFriends
        let friends = get3(8, "SteamFriends017");
        let persona: unsafe extern "C" fn(This) -> *const c_char = slot(friends, 0);
        assert_eq!(std::ffi::CStr::from_ptr(persona(friends)).to_str().unwrap(), "Tester");

        // ISteamUserStats: request, read, change, store.
        let stats = get3(13, "STEAMUSERSTATS_INTERFACE_VERSION012");
        let request: unsafe extern "C" fn(This) -> bool = slot(stats, 0);
        let get_int: unsafe extern "C" fn(This, *const c_char, *mut i32) -> bool = slot(stats, 1);
        let set_int: unsafe extern "C" fn(This, *const c_char, i32) -> bool = slot(stats, 3);
        let set_float: unsafe extern "C" fn(This, *const c_char, f32) -> bool = slot(stats, 4);
        let get_float: unsafe extern "C" fn(This, *const c_char, *mut f32) -> bool = slot(stats, 2);
        let get_ach: unsafe extern "C" fn(This, *const c_char, *mut bool) -> bool = slot(stats, 6);
        let set_ach: unsafe extern "C" fn(This, *const c_char) -> bool = slot(stats, 7);
        let store: unsafe extern "C" fn(This) -> bool = slot(stats, 10);
        drain(pipe);
        assert!(request(stats));
        let cbs = drain(pipe);
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].0, 1101, "UserStatsReceived_t");
        assert_eq!(cbs[0].1.len(), 20);
        assert_eq!(u64::from_le_bytes(cbs[0].1[0..8].try_into().unwrap()), u64::from(APP));
        assert_eq!(i32::from_le_bytes(cbs[0].1[8..12].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(cbs[0].1[12..20].try_into().unwrap()), ME);
        let mut v = 0;
        assert!(get_int(stats, c("kills").as_ptr(), &mut v));
        assert_eq!(v, 7);
        assert!(!get_int(stats, c("nope").as_ptr(), &mut v));
        let mut done = false;
        assert!(get_ach(stats, c("ACH_OLD").as_ptr(), &mut done) && done);
        assert!(get_ach(stats, c("ACH_WIN").as_ptr(), &mut done) && !done);
        assert!(set_int(stats, c("kills").as_ptr(), 8));
        assert!(set_float(stats, c("dist").as_ptr(), 2.5));
        let mut fl = 0f32;
        assert!(get_float(stats, c("dist").as_ptr(), &mut fl) && fl == 2.5);
        assert!(set_ach(stats, c("ACH_WIN").as_ptr()));
        assert!(store(stats));
        let cbs = drain(pipe);
        assert_eq!(cbs.iter().map(|c| c.0).collect::<Vec<_>>(), [1102, 1103]);
        assert_eq!(cbs[1].1.len(), 148);
        assert_eq!(&cbs[1].1[9..16], b"ACH_WIN");
        {
            let f = fake.lock().unwrap();
            assert_eq!(f.stores, 1);
            assert_eq!(f.stats["kills"], StatValue::Int(8));
            assert_eq!(f.stats["dist"], StatValue::Float(2.5));
            assert!(f.achievements.contains_key("ACH_WIN") && f.achievements.contains_key("ACH_OLD"));
        }
        // A second store announces no achievement again.
        assert!(store(stats));
        assert_eq!(drain(pipe).iter().map(|c| c.0).collect::<Vec<_>>(), [1102]);

        // Encrypted app ticket: a call result delivered through SteamAPICallCompleted_t.
        let req_enc: unsafe extern "C" fn(This, *const u8, i32) -> u64 = slot(su, 20);
        let get_enc: unsafe extern "C" fn(This, *mut u8, i32, *mut u32) -> bool = slot(su, 21);
        let call = req_enc(su, b"hi".as_ptr(), 2);
        assert_ne!(call, 0);
        let cbs = drain(pipe);
        assert_eq!(cbs.len(), 1);
        assert_eq!(cbs[0].0, 703);
        assert_eq!(u64::from_le_bytes(cbs[0].1[0..8].try_into().unwrap()), call);
        assert_eq!(i32::from_le_bytes(cbs[0].1[8..12].try_into().unwrap()), 154);
        let mut result = [0u8; 4];
        let mut failed = true;
        assert!(api_result(pipe, call, result.as_mut_ptr(), 4, 154, &mut failed));
        assert!(!failed);
        assert_eq!(i32::from_le_bytes(result), 1);
        assert!(!api_result(pipe, call, result.as_mut_ptr(), 4, 154, &mut failed), "a result is handed out once");
        let mut t = [0u8; 16];
        let mut tl = 0u32;
        assert!(get_enc(su, t.as_mut_ptr(), 16, &mut tl));
        assert_eq!(&t[..tl as usize], &[1, 2, 3]);

        // Auth session ticket.
        let auth: unsafe extern "C" fn(This, *mut u8, i32, *mut u32) -> u32 = slot(su, 13);
        let cancel: unsafe extern "C" fn(This, u32) = slot(su, 16);
        let h = auth(su, t.as_mut_ptr(), 16, &mut tl);
        assert_ne!(h, 0);
        assert_eq!(&t[..tl as usize], &[9, 8, 7, 6]);
        let cbs = drain(pipe);
        assert_eq!(cbs[0].0, 163);
        assert_eq!(u32::from_le_bytes(cbs[0].1[0..4].try_into().unwrap()), h);
        cancel(su, h);
        let mut small = [0u8; 2];
        assert_eq!(auth(su, small.as_mut_ptr(), 2, &mut tl), 0, "a ticket that does not fit is refused");

        // Interfaces we do not implement are safe to call.
        for ver in ["SteamInput006", "STEAMREMOTESTORAGE_INTERFACE_VERSION016", "SteamNetworkingSockets012"] {
            let o = get3(17, ver);
            assert!(!o.is_null(), "{ver}");
            for i in 0..60 {
                let f: unsafe extern "C" fn(This) -> usize = slot(o, i);
                assert_eq!(f(o), 0, "{ver} slot {i}");
            }
        }

        // Pipes can be released; callbacks on a released pipe are not served.
        let release: unsafe extern "C" fn(This, i32) -> bool = slot(client, 1);
        assert!(release(client, pipe));
        let mut msg = CallbackMsg { user: 0, callback: 0, param: std::ptr::null_mut(), size: 0 };
        assert!(!bget(pipe, &mut msg, std::ptr::null_mut()));
    }
    let _ = std::fs::remove_file(&sock);
}
