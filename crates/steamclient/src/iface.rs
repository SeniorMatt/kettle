//! The C++ interface objects games get from `ISteamClient::GetISteamXxx`.
//!
//! An interface object is a pointer to a pointer to a table of function pointers (the Itanium C++ ABI vtable of an
//! abstract class with no data members seen from outside). Slot `n` is the n-th virtual function in declaration
//! order (overloads are adjacent, in declaration order; there is no destructor slot in the Steamworks interfaces).
//! The first argument of every slot is the object pointer. On x86-64 System V and on i386 a C `extern "C"`
//! function with `this` as first parameter has the same calling convention as the member function.
//!
//! Tables are built by pushing slots in header order, so each builder below reads like the interface it mirrors.
//! A slot with no implementation gets [`zero`], which returns 0/false/NULL for any signature.

use crate::proto::{ERESULT_NO_CONNECTION, ERESULT_OK, Reply, Request, StatValue, hex_decode, hex_encode};
use crate::state::*;
use std::ffi::{CStr, c_char, c_void};
use std::time::{SystemTime, UNIX_EPOCH};

pub type This = *mut c_void;

// ---------------------------------------------------------------- helpers

/// # Safety
/// `p` is NULL or a NUL-terminated string.
pub unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Copy `s` with a NUL into `buf` (`len` bytes), truncating. False when there is no room at all.
///
/// # Safety
/// `buf` is NULL or has room for `len` bytes.
pub unsafe fn put_str(buf: *mut c_char, len: i32, s: &str) -> bool {
    if buf.is_null() || len <= 0 {
        return false;
    }
    let n = s.len().min(len as usize - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(s.as_ptr(), buf as *mut u8, n);
        *buf.add(n) = 0;
    }
    true
}

fn cs(s: &str) -> *const c_char {
    g().static_str(s)
}

fn now_secs() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

// ---------------------------------------------------------------- stubs

/// Any slot we don't implement: returns 0 / false / NULL for every signature.
pub extern "C" fn zero() -> usize {
    0
}
/// The one canonical address of [`zero`]. Rust does not promise a function a unique address across codegen units,
/// so tables and tests both take it from here (never inlined: one body, one address).
#[inline(never)]
pub fn zero_addr() -> usize {
    zero as *const () as usize
}
/// A `const char*` slot we don't implement: an empty string, never NULL.
pub extern "C" fn empty_str() -> *const c_char {
    cs("")
}
extern "C" fn minus_one() -> i32 {
    -1
}
/// A `bool` slot whose safe answer is "yes".
extern "C" fn ret_true() -> bool {
    true
}
extern "C" fn byte_ff() -> u8 {
    255
}

// ---------------------------------------------------------------- interface kinds

pub use crate::layout::{Kind, classify};
use crate::layout::{slots, stats_are_pushed};

/// Does `CreateInterface`/`Steam_IsKnownInterface` know this name? Every Steamworks interface we recognise is
/// answered (the ones without an implementation by a do-nothing object).
pub fn is_known(ver: &str) -> bool {
    classify(ver).is_some()
}

fn leak_table(mut t: Vec<usize>) -> *const usize {
    // Slack at the end: a game built against a newer header than ours must not run off the table.
    t.extend(std::iter::repeat_n(zero_addr(), 24));
    Box::leak(t.into_boxed_slice()).as_ptr()
}

fn new_object(table: *const usize) -> usize {
    Box::leak(Box::new(table)) as *mut *const usize as usize
}

/// A do-nothing table (unknown or unimplemented interface): 160 slots that return 0 / false / NULL.
fn null_table() -> *const usize {
    leak_table(vec![zero_addr(); 160])
}

/// The slot table of a known interface version: every method name gets its implementation or the do-nothing slot.
fn table_for(kind: Kind, n: u32) -> *const usize {
    let names = slots(kind, n);
    if kind == Kind::Stub {
        return null_table();
    }
    let t: Vec<usize> = names.iter().map(|name| resolve(kind, name)).collect();
    if kind == Kind::Input {
        // ISteamInput has 48 slots; only the first two mean something.
        let mut t = t;
        t.resize(64, zero_addr());
        return leak_table(t);
    }
    leak_table(t)
}

/// Never inlined, for the same reason as [`zero_addr`]: tests compare against what this returns.
/// The function behind method `name` of `kind`; [`zero`] when we do not implement it.
#[inline(never)]
pub fn resolve(kind: Kind, name: &str) -> usize {
    let f = |p: *const ()| p as usize;
    let z = f(zero as *const ());
    match kind {
        Kind::Client => match name {
            "CreateSteamPipe" => f(client_create_pipe as *const ()),
            "BReleaseSteamPipe" => f(client_release_pipe as *const ()),
            "ConnectToGlobalUser" => f(client_connect_user as *const ()),
            "CreateLocalUser" => f(client_create_local_user as *const ()),
            "GetISteamUtils" => f(client_get_utils as *const ()),
            "BShutdownIfAllPipesClosed" => f(client_shutdown_if_closed as *const ()),
            n if n.starts_with("GetISteam") => f(client_get_interface as *const ()),
            _ => z,
        },
        Kind::User => match name {
            "GetHSteamUser" => f(user_hsteam_user as *const ()),
            "BLoggedOn" => f(user_logged_on as *const ()),
            "GetSteamID" => f(user_steam_id as *const ()),
            "GetAuthSessionTicket" => f(user_auth_session_ticket as *const ()),
            "BeginAuthSession" => f(user_begin_auth_session as *const ()),
            "CancelAuthTicket" => f(user_cancel_auth_ticket as *const ()),
            "UserHasLicenseForApp" => f(user_has_license as *const ()),
            "RequestEncryptedAppTicket" => f(user_request_encrypted_ticket as *const ()),
            "GetEncryptedAppTicket" => f(user_get_encrypted_ticket as *const ()),
            _ => z,
        },
        Kind::Friends => match name {
            "GetPersonaName" => f(friends_persona_name as *const ()),
            "GetPersonaState" => f(friends_persona_state as *const ()),
            "GetFriendPersonaState" => f(friends_friend_state as *const ()),
            "GetFriendPersonaName" => f(friends_friend_name as *const ()),
            "GetFriendPersonaNameHistory" => f(empty_str as *const ()),
            "SetRichPresence" => f(ret_true as *const ()),
            _ => z,
        },
        Kind::Utils => match name {
            "GetSecondsSinceAppActive" | "GetSecondsSinceComputerActive" => f(utils_seconds_since_start as *const ()),
            "GetConnectedUniverse" => f(utils_universe as *const ()),
            "GetServerRealTime" => f(utils_server_real_time as *const ()),
            "GetIPCountry" => f(empty_str as *const ()),
            "GetCurrentBatteryPower" => f(byte_ff as *const ()),
            "GetAppID" => f(utils_app_id as *const ()),
            "IsAPICallCompleted" => f(utils_is_call_completed as *const ()),
            "GetAPICallFailureReason" => f(minus_one as *const ()),
            "GetAPICallResult" => f(utils_call_result as *const ()),
            "GetSteamUILanguage" => f(utils_ui_language as *const ()),
            _ => z,
        },
        Kind::Apps => match name {
            "BIsSubscribed" => f(apps_subscribed as *const ()),
            "GetCurrentGameLanguage" => f(apps_language as *const ()),
            "GetAvailableGameLanguages" => f(apps_available_languages as *const ()),
            "BIsSubscribedApp" | "BIsDlcInstalled" => f(apps_subscribed_app as *const ()),
            "GetAppInstallDir" => f(apps_install_dir as *const ()),
            "BIsAppInstalled" => f(apps_is_installed as *const ()),
            "GetAppOwner" => f(apps_owner as *const ()),
            "GetLaunchQueryParam" => f(empty_str as *const ()),
            "GetAppBuildId" => f(apps_build_id as *const ()),
            "GetLaunchCommandLine" => f(apps_launch_command_line as *const ()),
            _ => z,
        },
        Kind::UserStats => match name {
            "RequestCurrentStats" => f(us_request_current_stats as *const ()),
            "GetStatInt" => f(us_get_stat_int as *const ()),
            "GetStatFloat" => f(us_get_stat_float as *const ()),
            "SetStatInt" => f(us_set_stat_int as *const ()),
            "SetStatFloat" => f(us_set_stat_float as *const ()),
            "GetAchievement" => f(us_get_achievement as *const ()),
            "SetAchievement" => f(us_set_achievement as *const ()),
            "ClearAchievement" => f(us_clear_achievement as *const ()),
            "GetAchievementAndUnlockTime" => f(us_get_achievement_and_time as *const ()),
            "StoreStats" => f(us_store_stats as *const ()),
            "GetAchievementDisplayAttribute" | "GetAchievementName" => f(empty_str as *const ()),
            "ResetAllStats" => f(us_reset_all as *const ()),
            _ => z,
        },
        Kind::Input => match name {
            "Init" | "Shutdown" => f(ret_true as *const ()),
            _ => z,
        },
        Kind::Stub => z,
    }
}

/// The object for `ver`, created on first use. Unknown versions get a null object (and a log line, once): a
/// game never crashes on an interface we have not heard of, its calls just do nothing.
pub fn interface_for(ver: &str) -> usize {
    if let Some(p) = g().interfaces.get(ver) {
        return *p;
    }
    let class = classify(ver);
    let table = match class {
        Some((kind, n)) if kind != Kind::Stub => table_for(kind, n),
        other => {
            let first = g().warned.insert(ver.to_string());
            if first {
                let what = if other.is_some() { "is only a do-nothing object" } else { "is unknown" };
                log(&format!("interface {ver} {what}; calls on it do nothing"));
            }
            null_table()
        }
    };
    let obj = new_object(table);
    g().interfaces.insert(ver.to_string(), obj);
    // From ISteamUserStats013 on there is no RequestCurrentStats: the client sends the stats by itself.
    if let Some((Kind::UserStats, n)) = class
        && stats_are_pushed(n)
        && !us_request_current_stats(std::ptr::null_mut())
    {
        // Without an answer the game would wait for its stats for ever: tell it there is no connection.
        let mut cb = game_id().to_le_bytes().to_vec();
        cb.extend_from_slice(&ERESULT_NO_CONNECTION.to_le_bytes());
        cb.extend_from_slice(&identity().steam_id.to_le_bytes());
        g().push(CB_USER_STATS_RECEIVED, cb);
    }
    obj
}

// ---------------------------------------------------------------- ISteamClient

pub extern "C" fn client_create_pipe(_t: This) -> i32 {
    g().new_pipe()
}
pub extern "C" fn client_release_pipe(_t: This, pipe: i32) -> bool {
    g().pipes.remove(&pipe)
}
pub extern "C" fn client_connect_user(_t: This, pipe: i32) -> i32 {
    if g().pipes.contains(&pipe) { 1 } else { 0 }
}
extern "C" fn client_create_local_user(_t: This, pipe: *mut i32, _account_type: i32) -> i32 {
    if !pipe.is_null() {
        // SAFETY: the caller passes room for an HSteamPipe.
        unsafe { *pipe = g().new_pipe() };
    }
    1
}
/// The one getter every `GetISteamXxx(user, pipe, version)` slot shares: the version string decides.
extern "C" fn client_get_interface(_t: This, _user: i32, _pipe: i32, ver: *const c_char) -> usize {
    // SAFETY: `ver` is a C string from the game.
    let ver = unsafe { cstr(ver) };
    if verbose() {
        log(&format!("GetISteam* {ver}"));
    }
    interface_for(&ver)
}
/// `GetISteamUtils(pipe, version)` has one argument fewer than the other getters.
extern "C" fn client_get_utils(_t: This, _pipe: i32, ver: *const c_char) -> usize {
    // SAFETY: as above.
    let ver = unsafe { cstr(ver) };
    interface_for(&ver)
}
extern "C" fn client_shutdown_if_closed(_t: This) -> bool {
    g().pipes.is_empty()
}

// ---------------------------------------------------------------- ISteamUser

extern "C" fn user_hsteam_user(_t: This) -> i32 {
    1
}
extern "C" fn user_logged_on(_t: This) -> bool {
    identity().logged_on
}
extern "C" fn user_steam_id(_t: This) -> u64 {
    identity().steam_id
}
/// `GetAuthSessionTicket(void* pTicket, int cbMaxTicket, uint32* pcbTicket[, const SteamNetworkingIdentity*])`.
extern "C" fn user_auth_session_ticket(_t: This, buf: *mut u8, max: i32, out_len: *mut u32) -> u32 {
    let app = current_app();
    let fail = |out_len: *mut u32| {
        if !out_len.is_null() {
            // SAFETY: the game passes a valid out pointer or NULL.
            unsafe { *out_len = 0 };
        }
        0
    };
    let ticket = match rpc(Request::AuthTicket { app }) {
        Some(Reply::Ticket { eresult: ERESULT_OK, ticket }) => hex_decode(&ticket).unwrap_or_default(),
        _ => return fail(out_len),
    };
    if ticket.is_empty() || buf.is_null() || max < 0 || ticket.len() > max as usize {
        return fail(out_len);
    }
    // SAFETY: `buf` has room for `max` bytes and the ticket fits.
    unsafe {
        std::ptr::copy_nonoverlapping(ticket.as_ptr(), buf, ticket.len());
        if !out_len.is_null() {
            *out_len = ticket.len() as u32;
        }
    }
    let mut gl = g();
    let h = gl.new_ticket_handle();
    gl.auth_tickets.insert(h, ticket);
    let mut cb = h.to_le_bytes().to_vec();
    cb.extend_from_slice(&ERESULT_OK.to_le_bytes());
    gl.push(CB_GET_AUTH_SESSION_TICKET_RESPONSE, cb);
    h
}
/// `BeginAuthSession`: we do not validate other players' tickets, so the honest answer is "invalid ticket" (1).
extern "C" fn user_begin_auth_session() -> i32 {
    1
}
extern "C" fn user_cancel_auth_ticket(_t: This, h: u32) {
    g().auth_tickets.remove(&h);
}
/// `UserHasLicenseForApp(CSteamID, AppId_t)`: 0 has license, 1 does not, 2 no auth.
extern "C" fn user_has_license(_t: This, id: u64, app: u32) -> i32 {
    if id != identity().steam_id {
        return 2;
    }
    if owns(app) { 0 } else { 1 }
}
/// `RequestEncryptedAppTicket(void* pDataToInclude, int cbDataToInclude)`.
extern "C" fn user_request_encrypted_ticket(_t: This, data: *const u8, len: i32) -> u64 {
    let app = current_app();
    let userdata = if data.is_null() || len <= 0 {
        Vec::new()
    } else {
        // SAFETY: the game passes `len` readable bytes.
        unsafe { std::slice::from_raw_parts(data, len as usize) }.to_vec()
    };
    let (eresult, ticket) = match rpc(Request::EncryptedTicket { app, userdata: hex_encode(&userdata) }) {
        Some(Reply::Ticket { eresult, ticket }) => (eresult, hex_decode(&ticket).unwrap_or_default()),
        _ => (ERESULT_NO_CONNECTION, Vec::new()),
    };
    let mut gl = g();
    gl.encrypted_ticket = (eresult == ERESULT_OK).then_some(ticket);
    gl.complete_call(CB_ENCRYPTED_APP_TICKET_RESPONSE, eresult.to_le_bytes().to_vec())
}
extern "C" fn user_get_encrypted_ticket(_t: This, buf: *mut u8, max: i32, out_len: *mut u32) -> bool {
    let gl = g();
    let Some(t) = gl.encrypted_ticket.as_ref() else { return false };
    if !out_len.is_null() {
        // SAFETY: valid out pointer or NULL.
        unsafe { *out_len = t.len() as u32 };
    }
    if buf.is_null() || max < 0 || t.len() > max as usize {
        return false;
    }
    // SAFETY: `buf` has room for `max` bytes.
    unsafe { std::ptr::copy_nonoverlapping(t.as_ptr(), buf, t.len()) };
    true
}

// ---------------------------------------------------------------- ISteamUtils

extern "C" fn utils_seconds_since_start(_t: This) -> u32 {
    g().started.elapsed().as_secs() as u32
}
extern "C" fn utils_universe(_t: This) -> i32 {
    1 // k_EUniversePublic
}
extern "C" fn utils_server_real_time(_t: This) -> u32 {
    now_secs()
}
extern "C" fn utils_app_id(_t: This) -> u32 {
    current_app()
}
fn call_result(call: u64, buf: *mut u8, size: i32, expected: i32, failed: *mut bool) -> bool {
    if !failed.is_null() {
        // SAFETY: valid out pointer or NULL.
        unsafe { *failed = false };
    }
    let mut gl = g();
    let Some((id, data)) = gl.results.get(&call) else { return false };
    if (expected != 0 && *id != expected) || buf.is_null() || size < 0 || data.len() > size as usize {
        return false;
    }
    // SAFETY: `buf` has room for `size` bytes and the result fits.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len()) };
    gl.results.remove(&call);
    true
}
pub fn api_call_result(call: u64, buf: *mut u8, size: i32, expected: i32, failed: *mut bool) -> bool {
    call_result(call, buf, size, expected, failed)
}
extern "C" fn utils_is_call_completed(_t: This, call: u64, failed: *mut bool) -> bool {
    if !failed.is_null() {
        // SAFETY: valid out pointer or NULL.
        unsafe { *failed = false };
    }
    g().results.contains_key(&call)
}
extern "C" fn utils_call_result(
    _t: This,
    call: u64,
    buf: *mut u8,
    size: i32,
    expected: i32,
    failed: *mut bool,
) -> bool {
    call_result(call, buf, size, expected, failed)
}
extern "C" fn utils_ui_language(_t: This) -> *const c_char {
    let l = identity().language;
    cs(if l.is_empty() { "english" } else { l.as_str() })
}

// ---------------------------------------------------------------- ISteamApps

extern "C" fn apps_subscribed(_t: This) -> bool {
    let app = current_app();
    app != 0 && owns(app)
}
extern "C" fn apps_language(_t: This) -> *const c_char {
    let l = identity().language;
    cs(if l.is_empty() { "english" } else { l.as_str() })
}
extern "C" fn apps_available_languages(_t: This) -> *const c_char {
    cs("english")
}
extern "C" fn apps_subscribed_app(_t: This, app: u32) -> bool {
    owns(app)
}
extern "C" fn apps_install_dir(_t: This, app: u32, buf: *mut c_char, size: u32) -> u32 {
    let Some(dir) = app_info(app).install_dir else { return 0 };
    // SAFETY: `buf` has room for `size` bytes.
    if unsafe { put_str(buf, size.min(i32::MAX as u32) as i32, &dir) } { dir.len() as u32 + 1 } else { 0 }
}
extern "C" fn apps_is_installed(_t: This, app: u32) -> bool {
    app_info(app).install_dir.is_some()
}
extern "C" fn apps_owner(_t: This) -> u64 {
    identity().steam_id
}
extern "C" fn apps_build_id(_t: This) -> i32 {
    app_info(current_app()).build_id as i32
}
extern "C" fn apps_launch_command_line(_t: This, buf: *mut c_char, size: i32) -> i32 {
    // SAFETY: `buf` has room for `size` bytes.
    unsafe { put_str(buf, size, "") };
    0
}

// ---------------------------------------------------------------- ISteamFriends

extern "C" fn friends_persona_name(_t: This) -> *const c_char {
    let p = identity().persona;
    cs(&p)
}
extern "C" fn friends_persona_state(_t: This) -> i32 {
    if identity().logged_on { 1 } else { 0 }
}
extern "C" fn friends_friend_state(_t: This, id: u64) -> i32 {
    if id != 0 && id == identity().steam_id { 1 } else { 0 }
}
extern "C" fn friends_friend_name(_t: This, id: u64) -> *const c_char {
    let me = identity();
    if id != 0 && id == me.steam_id { cs(&me.persona) } else { cs("[unknown]") }
}

// ---------------------------------------------------------------- ISteamUserStats

fn game_id() -> u64 {
    u64::from(current_app())
}

extern "C" fn us_request_current_stats(_t: This) -> bool {
    let app = current_app();
    let me = identity();
    let Some(rep) = rpc(Request::GetStats { app }) else { return false };
    let mut gl = g();
    let eresult = match rep {
        Reply::Stats { eresult, stats, achievements } => {
            if eresult == ERESULT_OK {
                gl.stats.stats = stats;
                gl.stats.achievements = achievements;
                gl.stats.loaded = true;
            }
            eresult
        }
        _ => crate::proto::ERESULT_FAIL,
    };
    let mut cb = game_id().to_le_bytes().to_vec();
    cb.extend_from_slice(&eresult.to_le_bytes());
    cb.extend_from_slice(&me.steam_id.to_le_bytes());
    gl.push(CB_USER_STATS_RECEIVED, cb);
    true
}
extern "C" fn us_get_stat_int(_t: This, name: *const c_char, out: *mut i32) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    match (g().stats.stats.get(&name), out.is_null()) {
        (Some(StatValue::Int(v)), false) => {
            // SAFETY: valid out pointer.
            unsafe { *out = *v };
            true
        }
        _ => false,
    }
}
extern "C" fn us_get_stat_float(_t: This, name: *const c_char, out: *mut f32) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    match (g().stats.stats.get(&name), out.is_null()) {
        (Some(StatValue::Float(v)), false) => {
            // SAFETY: valid out pointer.
            unsafe { *out = *v };
            true
        }
        _ => false,
    }
}
extern "C" fn us_set_stat_int(_t: This, name: *const c_char, v: i32) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    if name.is_empty() {
        return false;
    }
    g().stats.stats.insert(name, StatValue::Int(v));
    true
}
extern "C" fn us_set_stat_float(_t: This, name: *const c_char, v: f32) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    if name.is_empty() {
        return false;
    }
    g().stats.stats.insert(name, StatValue::Float(v));
    true
}
extern "C" fn us_get_achievement(_t: This, name: *const c_char, out: *mut bool) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    if out.is_null() {
        return false;
    }
    // SAFETY: valid out pointer.
    unsafe { *out = g().stats.achievements.contains_key(&name) };
    true
}
extern "C" fn us_set_achievement(_t: This, name: *const c_char) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    if name.is_empty() {
        return false;
    }
    let mut gl = g();
    if !gl.stats.achievements.contains_key(&name) {
        gl.stats.achievements.insert(name.clone(), now_secs());
        gl.stats.fresh.insert(name);
    }
    true
}
extern "C" fn us_clear_achievement(_t: This, name: *const c_char) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    let mut gl = g();
    gl.stats.fresh.remove(&name);
    gl.stats.achievements.remove(&name);
    true
}
extern "C" fn us_get_achievement_and_time(_t: This, name: *const c_char, out: *mut bool, time: *mut u32) -> bool {
    // SAFETY: C string from the game.
    let name = unsafe { cstr(name) };
    if out.is_null() {
        return false;
    }
    let t = g().stats.achievements.get(&name).copied();
    // SAFETY: valid out pointers (`time` may be NULL).
    unsafe {
        *out = t.is_some();
        if !time.is_null() {
            *time = t.unwrap_or(0);
        }
    }
    true
}
fn achievement_stored(name: &str) -> Vec<u8> {
    // UserAchievementStored_t, 4-byte packed on Linux: u64 game, bool group, char[128] name, u32 cur, u32 max.
    let mut v = vec![0u8; 148];
    v[..8].copy_from_slice(&game_id().to_le_bytes());
    let n = name.len().min(127);
    v[9..9 + n].copy_from_slice(&name.as_bytes()[..n]);
    v
}
extern "C" fn us_store_stats(_t: This) -> bool {
    let app = current_app();
    let (stats, achievements, fresh) = {
        let gl = g();
        (gl.stats.stats.clone(), gl.stats.achievements.clone(), gl.stats.fresh.clone())
    };
    let Some(rep) = rpc(Request::StoreStats { app, stats, achievements }) else { return false };
    let eresult = match rep {
        Reply::Stored { eresult } => eresult,
        _ => crate::proto::ERESULT_FAIL,
    };
    let mut gl = g();
    let mut cb = game_id().to_le_bytes().to_vec();
    cb.extend_from_slice(&eresult.to_le_bytes());
    gl.push(CB_USER_STATS_STORED, cb);
    if eresult == ERESULT_OK {
        for name in &fresh {
            gl.push(CB_USER_ACHIEVEMENT_STORED, achievement_stored(name));
            gl.stats.fresh.remove(name);
        }
    }
    true
}
extern "C" fn us_reset_all(_t: This, achievements_too: bool) -> bool {
    let mut gl = g();
    gl.stats.stats.clear();
    if achievements_too {
        gl.stats.achievements.clear();
        gl.stats.fresh.clear();
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The function pointer in slot `i` of the table of `kind`/`n`.
    fn slot_of(kind: Kind, n: u32, i: usize) -> usize {
        let t = table_for(kind, n);
        // SAFETY: `table_for` leaks a table with at least `slots(kind, n).len()` entries plus slack.
        unsafe { *t.add(i) }
    }

    fn z() -> usize {
        zero_addr()
    }

    #[test]
    fn client_tables_put_the_functions_where_the_headers_do() {
        for n in 17..=23u32 {
            let names = slots(Kind::Client, n);
            let shutdown = names.iter().position(|s| *s == "BShutdownIfAllPipesClosed").unwrap();
            assert_eq!(slot_of(Kind::Client, n, shutdown), resolve(Kind::Client, "BShutdownIfAllPipesClosed"), "{n}");
            assert_eq!(slot_of(Kind::Client, n, 9), resolve(Kind::Client, "GetISteamUtils"));
            assert_eq!(slot_of(Kind::Client, n, 5), resolve(Kind::Client, "GetISteamFriends"));
            let run = names.iter().position(|s| *s == "RunFrame").unwrap();
            assert_eq!(slot_of(Kind::Client, n, run), z(), "RunFrame takes no version string: never the getter");
            // The slack after the last slot does nothing.
            assert_eq!(slot_of(Kind::Client, n, names.len() + 3), z());
        }
    }

    #[test]
    fn only_the_getters_of_every_client_version_take_a_version_string() {
        for n in 17..=23u32 {
            for (i, name) in slots(Kind::Client, n).iter().enumerate() {
                let is_getter = slot_of(Kind::Client, n, i) == resolve(Kind::Client, "GetISteamFriends");
                assert_eq!(is_getter, name.starts_with("GetISteam") && *name != "GetISteamUtils", "{n} {name}");
            }
        }
    }

    #[test]
    fn user_stats_013_is_shifted_by_the_missing_request() {
        let store = |n| slots(Kind::UserStats, n).iter().position(|s| *s == "StoreStats").unwrap();
        assert_eq!(slot_of(Kind::UserStats, 12, store(12)), resolve(Kind::UserStats, "StoreStats"));
        assert_eq!(slot_of(Kind::UserStats, 13, store(13)), resolve(Kind::UserStats, "StoreStats"));
        assert_eq!(slot_of(Kind::UserStats, 13, 0), resolve(Kind::UserStats, "GetStatInt"));
        assert_eq!(slot_of(Kind::UserStats, 11, 0), resolve(Kind::UserStats, "RequestCurrentStats"));
    }

    #[test]
    fn utils_011_loses_the_ip_port_slot() {
        assert_eq!(slot_of(Kind::Utils, 10, 9), resolve(Kind::Utils, "GetAppID"));
        assert_eq!(slot_of(Kind::Utils, 11, 8), resolve(Kind::Utils, "GetAppID"));
        assert_eq!(slot_of(Kind::Utils, 11, 9), z());
    }

    #[test]
    fn friends_018_moves_rich_presence_up() {
        assert_eq!(slot_of(Kind::Friends, 17, 43), resolve(Kind::Friends, "SetRichPresence"));
        assert_eq!(slot_of(Kind::Friends, 18, 41), resolve(Kind::Friends, "SetRichPresence"));
        assert_eq!(slot_of(Kind::Friends, 17, 0), resolve(Kind::Friends, "GetPersonaName"));
        assert_eq!(slot_of(Kind::Friends, 18, 1), resolve(Kind::Friends, "GetPersonaState"));
        assert_eq!(slot_of(Kind::Friends, 17, 2), resolve(Kind::Friends, "GetPersonaState"));
    }

    #[test]
    fn unknown_and_unimplemented_versions_get_a_do_nothing_object() {
        for ver in ["SteamNetworkingUtils004", "STEAMREMOTESTORAGE_INTERFACE_VERSION016", "SteamFutureThing001", ""] {
            let obj = interface_for(ver);
            assert_ne!(obj, 0, "{ver:?}");
            // SAFETY: an interface object is a pointer to a table with 160+ do-nothing entries.
            let table = unsafe { *(obj as *const *const usize) };
            let (first, last) = unsafe { (*table, *table.add(150)) };
            assert_eq!((first, last), (z(), z()));
            assert_eq!(interface_for(ver), obj, "the same object every time");
        }
        assert!(is_known("SteamNetworkingUtils004") && !is_known("SteamFutureThing001"));
    }

    #[test]
    fn input_says_ok_to_init() {
        assert_eq!(slot_of(Kind::Input, 6, 0), resolve(Kind::Input, "Init"));
        assert_eq!(slot_of(Kind::Input, 6, 1), resolve(Kind::Input, "Shutdown"));
        assert_eq!(slot_of(Kind::Input, 6, 3), z());
    }
}
