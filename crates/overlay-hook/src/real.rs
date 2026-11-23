//! Finding the real function behind each hook, and hooking `dlsym` so that games that look functions up by
//! name (SDL loads `glXSwapBuffers` from libGL with `dlsym`, which bypasses LD_PRELOAD) get ours too.

use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

/// One hooked function: our version and (once known) the real one.
pub struct Slot {
    pub name: &'static str,
    ours: *const (),
    real: AtomicUsize,
}

unsafe impl Sync for Slot {}

impl Slot {
    pub const fn new(name: &'static str, ours: *const ()) -> Slot {
        Slot { name, ours, real: AtomicUsize::new(0) }
    }

    /// The real function: remembered from a `dlsym` lookup or found behind us in the library search order.
    pub fn real(&self) -> Option<usize> {
        let p = self.real.load(Ordering::Acquire);
        if p != 0 {
            return Some(p);
        }
        let mut buf = [0u8; 64];
        let b = self.name.as_bytes();
        if b.len() >= buf.len() {
            return None;
        }
        buf[..b.len()].copy_from_slice(b);
        let p = unsafe { real_dlsym()(libc::RTLD_NEXT, buf.as_ptr().cast()) } as usize;
        if p != 0 && p != self.ours as usize {
            self.real.store(p, Ordering::Release);
            return Some(p);
        }
        None
    }

    pub fn ours(&self) -> *const c_void {
        self.ours.cast()
    }

    pub fn set_real(&self, p: usize) {
        if p != self.ours as usize {
            self.real.store(p, Ordering::Release);
        }
    }
}

type DlsymFn = unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void;

/// libc's own `dlsym` (ours replaces it for everyone else). It is found by version, because looking it up by
/// name would give us back ourselves.
fn real_dlsym() -> DlsymFn {
    static P: AtomicUsize = AtomicUsize::new(0);
    let p = P.load(Ordering::Acquire);
    if p != 0 {
        return unsafe { std::mem::transmute::<usize, DlsymFn>(p) };
    }
    let mut found = 0usize;
    for v in [c"GLIBC_2.34", c"GLIBC_2.2.5", c"GLIBC_2.17", c"GLIBC_2.0"] {
        let f = unsafe { libc::dlvsym(libc::RTLD_NEXT, c"dlsym".as_ptr(), v.as_ptr()) } as usize;
        if f != 0 && f != dlsym as *const () as usize {
            found = f;
            break;
        }
    }
    P.store(found, Ordering::Release);
    unsafe { std::mem::transmute::<usize, DlsymFn>(found) }
}

fn all_slots() -> impl Iterator<Item = &'static Slot> {
    crate::gl::slots::ALL.iter().chain(crate::sdl::slots::ALL.iter()).chain(crate::xlib::slots::ALL.iter()).copied()
}

pub fn find(name: &CStr) -> Option<&'static Slot> {
    let n = name.to_bytes();
    all_slots().find(|s| s.name.as_bytes() == n)
}

/// A symbol from the real library by name (not hooked).
pub fn lookup(name: &CStr) -> Option<usize> {
    let p = unsafe { real_dlsym()(libc::RTLD_DEFAULT, name.as_ptr()) } as usize;
    (p != 0).then_some(p)
}

/// # Safety
/// Called by the dynamic linker's users with a valid handle and a C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void {
    let real = real_dlsym();
    let p = unsafe { real(handle, name) };
    if p.is_null() || name.is_null() {
        return p;
    }
    // Once the overlay is off, lookups give the real functions (the hooks pass through anyway).
    let ours = crate::guard::run("dlsym", || {
        find(unsafe { CStr::from_ptr(name) }).map(|slot| {
            slot.set_real(p as usize);
            slot.ours as *mut c_void
        })
    });
    ours.flatten().unwrap_or(p)
}

/// A function from a library that may be loaded privately (so `RTLD_NEXT` cannot see it): behind us first,
/// then in each named library when it is already loaded.
pub fn sym_in(name: &CStr, libs: &[&CStr]) -> Option<usize> {
    let p = unsafe { real_dlsym()(libc::RTLD_NEXT, name.as_ptr()) } as usize;
    if p != 0 {
        return Some(p);
    }
    for l in libs {
        let h = unsafe { libc::dlopen(l.as_ptr(), libc::RTLD_NOLOAD | libc::RTLD_LAZY) };
        if !h.is_null() {
            let p = unsafe { real_dlsym()(h, name.as_ptr()) } as usize;
            if p != 0 {
                return Some(p);
            }
        }
    }
    None
}

/// Declare hooked functions: `hooks! { fn name(real; a: T, b: U) -> R { body using real(a, b) } ... }`.
/// Generates the exported symbol, a slot for each, and `slots::ALL`.
#[macro_export]
macro_rules! hooks {
    ($( fn $name:ident($real:ident; $($a:ident : $t:ty),* $(,)?) -> $r:ty $body:block )*) => {
        $(
            #[unsafe(no_mangle)]
            #[allow(non_snake_case, clippy::missing_safety_doc, unused_variables)]
            pub unsafe extern "C" fn $name($($a: $t),*) -> $r {
                let Some(p) = slots::$name.real() else {
                    return unsafe { ::std::mem::zeroed() };
                };
                let $real: unsafe extern "C" fn($($t),*) -> $r = unsafe { ::std::mem::transmute::<usize, _>(p) };
                // Our code runs guarded; on a panic (or once the overlay is off) the real function is called.
                match $crate::guard::run(stringify!($name), || -> $r { $body }) {
                    Some(r) => r,
                    None => unsafe { $real($($a),*) },
                }
            }
        )*
        pub mod slots {
            #![allow(non_upper_case_globals)]
            use $crate::real::Slot;
            $( pub static $name: Slot = Slot::new(stringify!($name), super::$name as *const ()); )*
            pub static ALL: &[&Slot] = &[ $( &$name ),* ];
        }
    };
}
