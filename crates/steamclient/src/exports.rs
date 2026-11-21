//! The functions `steamclient.so` exports. `libsteam_api.so` and Proton's `lsteamclient` look these up by name
//! with `dlsym`; Proton refuses to start Steamworks games if any of the first seven is missing.

#![allow(non_snake_case, clippy::missing_safety_doc)]

use crate::iface::{self, This, cstr};
use crate::state::*;
use std::ffi::{c_char, c_void};

/// `CallbackMsg_t` (`steam_api_common.h`): which user, which callback id, and where the bytes are.
#[repr(C)]
pub struct CallbackMsg {
    pub user: i32,
    pub callback: i32,
    pub param: *mut u8,
    pub size: i32,
}

/// `void* CreateInterface(const char* name, int* returnCode)`. Only the `SteamClientNNN` names are valid here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CreateInterface(name: *const c_char, return_code: *mut i32) -> *mut c_void {
    // SAFETY: `name` is a C string from the caller.
    let name = unsafe { cstr(name) };
    let ok = iface::is_known(&name) && name.starts_with("SteamClient");
    if verbose() {
        log(&format!("CreateInterface {name}: {}", if ok { "ok" } else { "unknown" }));
    }
    if !return_code.is_null() {
        // SAFETY: valid out pointer or NULL.
        unsafe { *return_code = if ok { 0 } else { 1 } };
    }
    if ok { iface::interface_for(&name) as *mut c_void } else { std::ptr::null_mut() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_BGetCallback(pipe: i32, msg: *mut CallbackMsg, call: *mut i32) -> bool {
    if msg.is_null() {
        return false;
    }
    let mut gl = g();
    if !gl.pipes.contains(&pipe) {
        return false;
    }
    let Some(cb) = gl.queue.pop_front() else { return false };
    let id = cb.id;
    let buf = gl.last.insert(cb.data);
    // SAFETY: `msg` is a caller-provided CallbackMsg_t; `buf` stays alive until Steam_FreeLastCallback.
    unsafe {
        *msg = CallbackMsg { user: 1, callback: id, param: buf.as_mut_ptr(), size: buf.len() as i32 };
        if !call.is_null() {
            *call = 0;
        }
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_FreeLastCallback(_pipe: i32) -> bool {
    g().last = None;
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_GetAPICallResult(
    _pipe: i32,
    call: u64,
    buf: *mut u8,
    size: i32,
    expected: i32,
    failed: *mut bool,
) -> bool {
    iface::api_call_result(call, buf, size, expected, failed)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_ReleaseThreadLocalMemory(_thread_exit: i32) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_IsKnownInterface(name: *const c_char) -> bool {
    // SAFETY: C string from the caller.
    iface::is_known(&unsafe { cstr(name) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_NotifyMissingInterface(_pipe: i32, name: *const c_char) {
    // SAFETY: C string from the caller.
    log(&format!("the game reports a missing interface: {}", unsafe { cstr(name) }));
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_CreateSteamPipe() -> i32 {
    g().new_pipe()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_BReleaseSteamPipe(pipe: i32) -> bool {
    g().pipes.remove(&pipe)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_ConnectToGlobalUser(pipe: i32) -> i32 {
    iface::client_connect_user(std::ptr::null_mut::<c_void>() as This, pipe)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_CreateGlobalUser(pipe: *mut i32) -> i32 {
    let p = g().new_pipe();
    if !pipe.is_null() {
        // SAFETY: valid out pointer.
        unsafe { *pipe = p };
    }
    1
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_ReleaseUser(_pipe: i32, _user: i32) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_BLoggedOn(_pipe: i32, _user: i32) -> bool {
    identity().logged_on
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Steam_GetHSteamUserCurrent() -> i32 {
    1
}

// Crash reporting: games call these during start-up; we collect nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Breakpad_SteamMiniDumpInit(_a: u32, _b: *const c_char, _c: *const c_char) {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Breakpad_SteamSetAppID(_app: u32) {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Breakpad_SteamSetSteamID(_id: u64) {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Breakpad_SteamWriteMiniDumpSetComment(_comment: *const c_char) {}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Breakpad_SteamWriteMiniDumpUsingExceptionInfoWithBuildId(_a: u32, _b: *mut c_void, _c: u32) {}
