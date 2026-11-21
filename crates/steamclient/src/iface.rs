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
/// A `const char*` slot we don't implement: an empty string, never NULL.
pub extern "C" fn empty_str() -> *const c_char {
    cs("")
}
extern "C" fn minus_one() -> i32 {
    -1
}
extern "C" fn byte_ff() -> u8 {
    255
}

// ---------------------------------------------------------------- interface kinds

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Client,
    User,
    Friends,
    Utils,
    Apps,
    UserStats,
}

fn num_after(ver: &str, prefix: &str) -> Option<u32> {
    let rest = ver.strip_prefix(prefix)?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// The kind and number of a version string we implement. Everything else gets the null object.
pub fn classify(ver: &str) -> Option<(Kind, u32)> {
    let table: [(&str, Kind, std::ops::RangeInclusive<u32>); 6] = [
        ("SteamClient", Kind::Client, 17..=21),
        ("SteamUser", Kind::User, 19..=23),
        ("SteamFriends", Kind::Friends, 15..=18),
        ("SteamUtils", Kind::Utils, 9..=10),
        ("STEAMAPPS_INTERFACE_VERSION", Kind::Apps, 6..=8),
        ("STEAMUSERSTATS_INTERFACE_VERSION", Kind::UserStats, 11..=13),
    ];
    for (prefix, kind, range) in table {
        if let Some(n) = num_after(ver, prefix)
            && range.contains(&n)
        {
            return Some((kind, n));
        }
    }
    None
}

/// Does `CreateInterface`/`Steam_IsKnownInterface` know this name?
pub fn is_known(ver: &str) -> bool {
    classify(ver).is_some()
}

fn leak_table(mut t: Vec<usize>) -> *const usize {
    // Slack at the end: a game built against a newer header than ours must not run off the table.
    t.extend(std::iter::repeat_n(zero as *const () as usize, 24));
    Box::leak(t.into_boxed_slice()).as_ptr()
}

fn new_object(table: *const usize) -> usize {
    Box::leak(Box::new(table)) as *mut *const usize as usize
}

/// The object for `ver`, created on first use. Unknown versions get a null object (and a log line, once).
pub fn interface_for(ver: &str) -> usize {
    if let Some(p) = g().interfaces.get(ver) {
        return *p;
    }
    let table = match classify(ver) {
        Some((Kind::Client, n)) => client_table(n),
        Some((Kind::User, n)) => user_table(n),
        Some((Kind::Friends, _)) => friends_table(),
        Some((Kind::Utils, _)) => utils_table(),
        Some((Kind::Apps, _)) => apps_table(),
        Some((Kind::UserStats, _)) => user_stats_table(),
        None => {
            let first = g().warned.insert(ver.to_string());
            if first {
                log(&format!("interface {ver} is not implemented; calls on it do nothing"));
            }
            leak_table(vec![zero as *const () as usize; 160])
        }
    };
    let obj = new_object(table);
    g().interfaces.insert(ver.to_string(), obj);
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

fn client_table(n: u32) -> *const usize {
    let get = client_get_interface as *const () as usize;
    let mut t = vec![
        client_create_pipe as *const () as usize,       // 0 CreateSteamPipe
        client_release_pipe as *const () as usize,      // 1 BReleaseSteamPipe
        client_connect_user as *const () as usize,      // 2 ConnectToGlobalUser
        client_create_local_user as *const () as usize, // 3 CreateLocalUser
        zero as *const () as usize,                     // 4 ReleaseUser
        get,                                            // 5 GetISteamUser
        get,                                            // 6 GetISteamGameServer
        zero as *const () as usize,                     // 7 SetLocalIPBinding
        get,                                            // 8 GetISteamFriends
        client_get_utils as *const () as usize,         // 9 GetISteamUtils
        get,                                            // 10 GetISteamMatchmaking
        get,                                            // 11 GetISteamMatchmakingServers
        get,                                            // 12 GetISteamGenericInterface
        get,                                            // 13 GetISteamUserStats
        get,                                            // 14 GetISteamGameServerStats
        get,                                            // 15 GetISteamApps
        get,                                            // 16 GetISteamNetworking
        get,                                            // 17 GetISteamRemoteStorage
        get,                                            // 18 GetISteamScreenshots
    ];
    if n >= 20 {
        t.push(get); // GetISteamGameSearch (layout to verify against the 020/021 headers)
    }
    t.extend([
        zero as *const () as usize,                      // GetIPCCallCount
        zero as *const () as usize,                      // SetWarningMessageHook
        client_shutdown_if_closed as *const () as usize, // BShutdownIfAllPipesClosed
        get,                                             // GetISteamHTTP
        get,                                             // GetISteamUnifiedMessages (deprecated)
        get,                                             // GetISteamController
        get,                                             // GetISteamUGC
        get,                                             // GetISteamAppList
        get,                                             // GetISteamMusic
        get,                                             // GetISteamMusicRemote
        get,                                             // GetISteamHTMLSurface
        zero as *const () as usize,                      // Set_SteamAPI_CPostAPIResultInProcess (deprecated)
        zero as *const () as usize,                      // Remove_SteamAPI_CPostAPIResultInProcess (deprecated)
        zero as *const () as usize,                      // Set_SteamAPI_CCheckCallbackRegisteredInProcess
        get,                                             // GetISteamInventory
        get,                                             // GetISteamVideo
        get,                                             // GetISteamParentalSettings
        get,                                             // GetISteamInput
        get,                                             // GetISteamParties
        get,                                             // GetISteamRemotePlay
        zero as *const () as usize,                      // DestroyAllInterfaces
    ]);
    leak_table(t)
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

fn user_table(n: u32) -> *const usize {
    let z = zero as *const () as usize;
    let mut t = vec![
        user_hsteam_user as *const () as usize,         // 0 GetHSteamUser
        user_logged_on as *const () as usize,           // 1 BLoggedOn
        user_steam_id as *const () as usize,            // 2 GetSteamID
        z,                                              // 3 InitiateGameConnection_DEPRECATED
        z,                                              // 4 TerminateGameConnection_DEPRECATED
        z,                                              // 5 TrackAppUsageEvent
        z,                                              // 6 GetUserDataFolder
        z,                                              // 7 StartVoiceRecording
        z,                                              // 8 StopVoiceRecording
        z,                                              // 9 GetAvailableVoice
        z,                                              // 10 GetVoice
        z,                                              // 11 DecompressVoice
        z,                                              // 12 GetVoiceOptimalSampleRate
        user_auth_session_ticket as *const () as usize, // 13 GetAuthSessionTicket
    ];
    if n >= 23 {
        t.push(z); // GetAuthTicketForWebApi (layout to verify against the 023 header)
    }
    t.extend([
        user_begin_auth_session as *const () as usize,       // BeginAuthSession
        z,                                                   // EndAuthSession
        user_cancel_auth_ticket as *const () as usize,       // CancelAuthTicket
        user_has_license as *const () as usize,              // UserHasLicenseForApp
        z,                                                   // BIsBehindNAT
        z,                                                   // AdvertiseGame
        user_request_encrypted_ticket as *const () as usize, // RequestEncryptedAppTicket
        user_get_encrypted_ticket as *const () as usize,     // GetEncryptedAppTicket
        z,                                                   // GetGameBadgeLevel
        z,                                                   // GetPlayerSteamLevel
        z,                                                   // RequestStoreAuthURL
        z,                                                   // BIsPhoneVerified
        z,                                                   // BIsTwoFactorEnabled
        z,                                                   // BIsPhoneIdentifying
        z,                                                   // BIsPhoneRequiringVerification
        z,                                                   // GetMarketEligibility
        z,                                                   // GetDurationControl
        z,                                                   // BSetDurationControlOnlineState
    ]);
    leak_table(t)
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

fn utils_table() -> *const usize {
    let z = zero as *const () as usize;
    leak_table(vec![
        utils_seconds_since_start as *const () as usize, // 0 GetSecondsSinceAppActive
        utils_seconds_since_start as *const () as usize, // 1 GetSecondsSinceComputerActive
        utils_universe as *const () as usize,            // 2 GetConnectedUniverse
        utils_server_real_time as *const () as usize,    // 3 GetServerRealTime
        empty_str as *const () as usize,                 // 4 GetIPCountry
        z,                                               // 5 GetImageSize
        z,                                               // 6 GetImageRGBA
        z,                                               // 7 GetCSERIPPort
        byte_ff as *const () as usize,                   // 8 GetCurrentBatteryPower (255 = on AC)
        utils_app_id as *const () as usize,              // 9 GetAppID
        z,                                               // 10 SetOverlayNotificationPosition
        utils_is_call_completed as *const () as usize,   // 11 IsAPICallCompleted
        minus_one as *const () as usize,                 // 12 GetAPICallFailureReason (-1 = none)
        utils_call_result as *const () as usize,         // 13 GetAPICallResult
        z,                                               // 14 RunFrame
        z,                                               // 15 GetIPCCallCount
        z,                                               // 16 SetWarningMessageHook
        z,                                               // 17 IsOverlayEnabled (false: Kettle's overlay is separate)
        z,                                               // 18 BOverlayNeedsPresent
        z,                                               // 19 CheckFileSignature
        z,                                               // 20 ShowGamepadTextInput
        z,                                               // 21 GetEnteredGamepadTextLength
        z,                                               // 22 GetEnteredGamepadTextInput
        utils_ui_language as *const () as usize,         // 23 GetSteamUILanguage
        z,                                               // 24 IsSteamRunningInVR
        z,                                               // 25 SetOverlayNotificationInset
        z,                                               // 26 IsSteamInBigPictureMode
        z,                                               // 27 StartVRDashboard
        z,                                               // 28 IsVRHeadsetStreamingEnabled
        z,                                               // 29 SetVRHeadsetStreamingEnabled
        z,                                               // 30 IsSteamChinaLauncher
        z,                                               // 31 InitFilterText
        z,                                               // 32 FilterText
        z,                                               // 33 GetIPv6ConnectivityState
        z,                                               // 34 IsSteamRunningOnSteamDeck
    ])
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

fn apps_table() -> *const usize {
    let z = zero as *const () as usize;
    leak_table(vec![
        apps_subscribed as *const () as usize,          // 0 BIsSubscribed
        z,                                              // 1 BIsLowViolence
        z,                                              // 2 BIsCybercafe
        z,                                              // 3 BIsVACBanned
        apps_language as *const () as usize,            // 4 GetCurrentGameLanguage
        apps_available_languages as *const () as usize, // 5 GetAvailableGameLanguages
        apps_subscribed_app as *const () as usize,      // 6 BIsSubscribedApp
        apps_subscribed_app as *const () as usize,      // 7 BIsDlcInstalled (owned; installed state is not tracked yet)
        z,                                              // 8 GetEarliestPurchaseUnixTime
        z,                                              // 9 BIsSubscribedFromFreeWeekend
        z,                                              // 10 GetDLCCount
        z,                                              // 11 BGetDLCDataByIndex
        z,                                              // 12 InstallDLC
        z,                                              // 13 UninstallDLC
        z,                                              // 14 RequestAppProofOfPurchaseKey
        z,                                              // 15 GetCurrentBetaName
        z,                                              // 16 MarkContentCorrupt
        z,                                              // 17 GetInstalledDepots
        apps_install_dir as *const () as usize,         // 18 GetAppInstallDir
        apps_is_installed as *const () as usize,        // 19 BIsAppInstalled
        apps_owner as *const () as usize,               // 20 GetAppOwner
        empty_str as *const () as usize,                // 21 GetLaunchQueryParam
        z,                                              // 22 GetDlcDownloadProgress
        apps_build_id as *const () as usize,            // 23 GetAppBuildId
        z,                                              // 24 RequestAllProofOfPurchaseKeys
        z,                                              // 25 GetFileDetails
        apps_launch_command_line as *const () as usize, // 26 GetLaunchCommandLine
        z,                                              // 27 BIsSubscribedFromFamilySharing
        z,                                              // 28 BIsTimedTrial
        z,                                              // 29 SetDlcContext
    ])
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

fn friends_table() -> *const usize {
    let z = zero as *const () as usize;
    leak_table(vec![
        friends_persona_name as *const () as usize,  // 0 GetPersonaName
        z,                                           // 1 SetPersonaName
        friends_persona_state as *const () as usize, // 2 GetPersonaState
        z,                                           // 3 GetFriendCount
        z,                                           // 4 GetFriendByIndex
        z,                                           // 5 GetFriendRelationship
        friends_friend_state as *const () as usize,  // 6 GetFriendPersonaState
        friends_friend_name as *const () as usize,   // 7 GetFriendPersonaName
        empty_str as *const () as usize,             // 8 GetFriendGamePlayed (sret in real headers; not supported)
        empty_str as *const () as usize,             // 9 GetFriendPersonaNameHistory
        z,                                           // 10 GetFriendSteamLevel
        empty_str as *const () as usize,             // 11 GetPlayerNickname
    ])
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

fn user_stats_table() -> *const usize {
    let z = zero as *const () as usize;
    leak_table(vec![
        us_request_current_stats as *const () as usize,    // 0 RequestCurrentStats
        us_get_stat_int as *const () as usize,             // 1 GetStat(int32*)
        us_get_stat_float as *const () as usize,           // 2 GetStat(float*)
        us_set_stat_int as *const () as usize,             // 3 SetStat(int32)
        us_set_stat_float as *const () as usize,           // 4 SetStat(float)
        z,                                                 // 5 UpdateAvgRateStat
        us_get_achievement as *const () as usize,          // 6 GetAchievement
        us_set_achievement as *const () as usize,          // 7 SetAchievement
        us_clear_achievement as *const () as usize,        // 8 ClearAchievement
        us_get_achievement_and_time as *const () as usize, // 9 GetAchievementAndUnlockTime
        us_store_stats as *const () as usize,              // 10 StoreStats
        z,                                                 // 11 GetAchievementIcon
        empty_str as *const () as usize,                   // 12 GetAchievementDisplayAttribute
        z,                                                 // 13 IndicateAchievementProgress
        z,                                                 // 14 GetNumAchievements
        empty_str as *const () as usize,                   // 15 GetAchievementName
        z,                                                 // 16 RequestUserStats
        z,                                                 // 17 GetUserStat(int32*)
        z,                                                 // 18 GetUserStat(float*)
        z,                                                 // 19 GetUserAchievement
        z,                                                 // 20 GetUserAchievementAndUnlockTime
        us_reset_all as *const () as usize,                // 21 ResetAllStats
    ])
}
