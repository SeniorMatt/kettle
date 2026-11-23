//! A small SDL2 program that behaves like DDNet for input: OpenGL window, relative mouse mode, window grab,
//! hidden cursor. It writes what it receives to `$TEST_LOG` so a script can check what the game saw while the
//! overlay was open (nothing) and after (everything again). Links SDL2 normally, so LD_PRELOAD hooks apply
//! exactly as for a real game. Not part of the shipped program.

use std::ffi::{CString, c_char, c_int, c_void};
use std::io::Write;

#[link(name = "libSDL2-2.0.so.0", kind = "dylib", modifiers = "+verbatim")]
unsafe extern "C" {
    fn SDL_Init(flags: u32) -> c_int;
    fn SDL_CreateWindow(title: *const c_char, x: c_int, y: c_int, w: c_int, h: c_int, flags: u32) -> *mut c_void;
    fn SDL_GL_CreateContext(win: *mut c_void) -> *mut c_void;
    fn SDL_GL_SwapWindow(win: *mut c_void);
    fn SDL_GL_GetProcAddress(name: *const c_char) -> *mut c_void;
    fn SDL_SetRelativeMouseMode(on: c_int) -> c_int;
    fn SDL_GetRelativeMouseMode() -> c_int;
    fn SDL_SetWindowGrab(win: *mut c_void, on: c_int);
    fn SDL_GetWindowGrab(win: *mut c_void) -> c_int;
    fn SDL_ShowCursor(toggle: c_int) -> c_int;
    fn SDL_PollEvent(ev: *mut u8) -> c_int;
    fn SDL_Delay(ms: u32);
    fn SDL_GL_SetSwapInterval(i: c_int) -> c_int;
}

fn main() {
    let log_path = std::env::var("TEST_LOG").unwrap_or_else(|_| "/tmp/sdl_test.log".into());
    let quit_path = std::env::var("TEST_QUIT").unwrap_or_else(|_| "/tmp/sdl_test.quit".into());
    let mut log = std::fs::File::create(&log_path).expect("log file");
    unsafe {
        assert_eq!(SDL_Init(0x20), 0, "SDL_Init");
        let title = CString::new("sdl_test").unwrap();
        // SDL_WINDOW_OPENGL = 2
        let win = SDL_CreateWindow(title.as_ptr(), 100, 100, 640, 480, 2);
        assert!(!win.is_null(), "window");
        let ctx = SDL_GL_CreateContext(win);
        assert!(!ctx.is_null(), "context");
        SDL_GL_SetSwapInterval(1);
        let clear_color: unsafe extern "C" fn(f32, f32, f32, f32) =
            std::mem::transmute(SDL_GL_GetProcAddress(c"glClearColor".as_ptr()));
        let clear: unsafe extern "C" fn(u32) = std::mem::transmute(SDL_GL_GetProcAddress(c"glClear".as_ptr()));
        SDL_SetRelativeMouseMode(1);
        SDL_SetWindowGrab(win, 1);
        SDL_ShowCursor(0);
        writeln!(log, "ready").unwrap();
        log.flush().unwrap();
        let mut frame = 0u32;
        let mut ev = [0u8; 56];
        let start = std::time::Instant::now();
        while !std::path::Path::new(&quit_path).exists() && start.elapsed().as_secs() < 120 {
            while SDL_PollEvent(ev.as_mut_ptr()) != 0 {
                let ty = u32::from_le_bytes(ev[0..4].try_into().unwrap());
                let i32_at = |o: usize| i32::from_le_bytes(ev[o..o + 4].try_into().unwrap());
                match ty {
                    0x300 | 0x301 => {
                        writeln!(log, "key {} scancode={}", if ty == 0x300 { "down" } else { "up" }, i32_at(16)).unwrap()
                    }
                    0x401 | 0x402 => writeln!(
                        log,
                        "button {} {}",
                        if ty == 0x401 { "down" } else { "up" },
                        ev[16]
                    )
                    .unwrap(),
                    0x400 => writeln!(log, "motion xrel={} yrel={}", i32_at(28), i32_at(32)).unwrap(),
                    0x100 => {
                        writeln!(log, "quit").unwrap();
                        return;
                    }
                    _ => {}
                }
            }
            // What the game believes about itself (stays "1 1 0" whatever the overlay does).
            if frame % 30 == 0 {
                writeln!(
                    log,
                    "state rel={} grab={} cursor={}",
                    SDL_GetRelativeMouseMode(),
                    SDL_GetWindowGrab(win),
                    SDL_ShowCursor(-1)
                )
                .unwrap();
            }
            log.flush().unwrap();
            // A dark green background that changes a little, so a screenshot shows the game is running.
            clear_color(0.05, 0.25 + (frame % 60) as f32 / 600.0, 0.1, 1.0);
            clear(0x4000);
            SDL_GL_SwapWindow(win);
            SDL_Delay(8);
            frame += 1;
        }
    }
}
