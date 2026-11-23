//! OpenGL: draws the overlay with egui_glow just before the game's buffer swap, and puts every piece of GL
//! state it touched back, so the game cannot tell.

use crate::hooks;
use crate::real;
use crate::state::{self, Claim};
use egui_glow::Painter;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_uchar, c_uint, c_ulong, c_void};
use std::sync::Arc;

type Fp = *const c_void;

/// The raw GL functions needed to save and restore state (egui_glow does the drawing).
struct Raw {
    get_integerv: unsafe extern "C" fn(c_uint, *mut c_int),
    get_booleanv: unsafe extern "C" fn(c_uint, *mut c_uchar),
    get_error: unsafe extern "C" fn() -> c_uint,
    is_enabled: unsafe extern "C" fn(c_uint) -> c_uchar,
    enable: unsafe extern "C" fn(c_uint),
    disable: unsafe extern "C" fn(c_uint),
    bind_buffer: unsafe extern "C" fn(c_uint, c_uint),
    use_program: unsafe extern "C" fn(c_uint),
    active_texture: unsafe extern "C" fn(c_uint),
    bind_texture: unsafe extern "C" fn(c_uint, c_uint),
    blend_func_separate: unsafe extern "C" fn(c_uint, c_uint, c_uint, c_uint),
    blend_equation_separate: unsafe extern "C" fn(c_uint, c_uint),
    scissor: unsafe extern "C" fn(c_int, c_int, c_int, c_int),
    viewport: unsafe extern "C" fn(c_int, c_int, c_int, c_int),
    color_mask: unsafe extern "C" fn(c_uchar, c_uchar, c_uchar, c_uchar),
    pixel_storei: unsafe extern "C" fn(c_uint, c_int),
    bind_vertex_array: Option<unsafe extern "C" fn(c_uint)>,
    bind_framebuffer: Option<unsafe extern "C" fn(c_uint, c_uint)>,
    bind_sampler: Option<unsafe extern "C" fn(c_uint, c_uint)>,
    polygon_mode: Option<unsafe extern "C" fn(c_uint, c_uint)>,
}

const CAPS: [c_uint; 13] = [
    0x0BE2, // BLEND
    0x0B71, // DEPTH_TEST
    0x0B44, // CULL_FACE
    0x0C11, // SCISSOR_TEST
    0x0B90, // STENCIL_TEST
    0x0BD0, // DITHER
    0x8DB9, // FRAMEBUFFER_SRGB
    0x0BF2, // COLOR_LOGIC_OP
    0x8037, // POLYGON_OFFSET_FILL
    0x809E, // SAMPLE_ALPHA_TO_COVERAGE
    0x80A0, // SAMPLE_COVERAGE
    0x809D, // MULTISAMPLE
    0x8F9D, // PRIMITIVE_RESTART
];
/// The ones switched off while the overlay draws (the others are set by the painter or harmless).
const OFF: [usize; 9] = [1, 2, 4, 6, 7, 8, 9, 10, 12];

const GL_ARRAY_BUFFER: c_uint = 0x8892;
const GL_ELEMENT_ARRAY_BUFFER: c_uint = 0x8893;
const GL_PIXEL_UNPACK_BUFFER: c_uint = 0x88EC;
const GL_TEXTURE_2D: c_uint = 0x0DE1;
const GL_TEXTURE0: c_uint = 0x84C0;
const GL_FRAMEBUFFER: c_uint = 0x8D40;
const GL_DRAW_FRAMEBUFFER: c_uint = 0x8CA9;
const GL_READ_FRAMEBUFFER: c_uint = 0x8CA8;

/// Everything saved before the overlay draws.
#[derive(Default)]
struct Saved {
    array_buffer: i32,
    element_buffer: i32,
    vao: i32,
    program: i32,
    active_texture: i32,
    texture0: i32,
    sampler0: i32,
    unpack_buffer: i32,
    draw_fb: i32,
    read_fb: i32,
    blend: [i32; 6],
    scissor: [i32; 4],
    viewport: [i32; 4],
    unpack: [i32; 4],
    color_mask: [u8; 4],
    polygon_mode: [i32; 2],
    caps: [bool; 13],
}

impl Raw {
    unsafe fn load(get: &dyn Fn(&CStr) -> Fp) -> Option<Raw> {
        macro_rules! f {
            ($n:literal) => {{
                let p = get($n);
                if p.is_null() {
                    return None;
                }
                unsafe { std::mem::transmute(p) }
            }};
        }
        macro_rules! o {
            ($n:literal) => {{
                let p = get($n);
                if p.is_null() { None } else { Some(unsafe { std::mem::transmute(p) }) }
            }};
        }
        Some(Raw {
            get_integerv: f!(c"glGetIntegerv"),
            get_booleanv: f!(c"glGetBooleanv"),
            get_error: f!(c"glGetError"),
            is_enabled: f!(c"glIsEnabled"),
            enable: f!(c"glEnable"),
            disable: f!(c"glDisable"),
            bind_buffer: f!(c"glBindBuffer"),
            use_program: f!(c"glUseProgram"),
            active_texture: f!(c"glActiveTexture"),
            bind_texture: f!(c"glBindTexture"),
            blend_func_separate: f!(c"glBlendFuncSeparate"),
            blend_equation_separate: f!(c"glBlendEquationSeparate"),
            scissor: f!(c"glScissor"),
            viewport: f!(c"glViewport"),
            color_mask: f!(c"glColorMask"),
            pixel_storei: f!(c"glPixelStorei"),
            bind_vertex_array: o!(c"glBindVertexArray"),
            bind_framebuffer: o!(c"glBindFramebuffer"),
            bind_sampler: o!(c"glBindSampler"),
            polygon_mode: o!(c"glPolygonMode"),
        })
    }

    unsafe fn int(&self, e: c_uint) -> i32 {
        let mut v = 0;
        unsafe { (self.get_integerv)(e, &mut v) };
        v
    }

    unsafe fn save(&self) -> Saved {
        let mut s = Saved::default();
        unsafe {
            while (self.get_error)() != 0 {}
            s.array_buffer = self.int(0x8894);
            s.vao = if self.bind_vertex_array.is_some() { self.int(0x85B5) } else { 0 };
            s.element_buffer = self.int(0x8895);
            s.program = self.int(0x8B8D);
            s.active_texture = self.int(0x84E0);
            (self.active_texture)(GL_TEXTURE0);
            s.texture0 = self.int(0x8069);
            s.sampler0 = if self.bind_sampler.is_some() { self.int(0x8919) } else { 0 };
            s.unpack_buffer = self.int(0x88EF);
            if self.bind_framebuffer.is_some() {
                s.draw_fb = self.int(0x8CA6);
                s.read_fb = self.int(0x8CAA);
            }
            for (i, e) in [0x80C9, 0x80C8, 0x80CB, 0x80CA, 0x8009, 0x883D].into_iter().enumerate() {
                s.blend[i] = self.int(e);
            }
            (self.get_integerv)(0x0C10, s.scissor.as_mut_ptr());
            (self.get_integerv)(0x0BA2, s.viewport.as_mut_ptr());
            for (i, e) in [0x0CF5, 0x0CF2, 0x0CF4, 0x0CF3].into_iter().enumerate() {
                s.unpack[i] = self.int(e);
            }
            (self.get_booleanv)(0x0C23, s.color_mask.as_mut_ptr());
            if self.polygon_mode.is_some() {
                (self.get_integerv)(0x0B40, s.polygon_mode.as_mut_ptr());
            }
            for (i, c) in CAPS.into_iter().enumerate() {
                s.caps[i] = (self.is_enabled)(c) != 0;
            }
            while (self.get_error)() != 0 {}
        }
        s
    }

    /// The state the painter expects: nothing from the game that changes how it draws.
    unsafe fn prepare(&self) {
        unsafe {
            for i in OFF {
                (self.disable)(CAPS[i]);
            }
            if let Some(bind) = self.bind_framebuffer {
                bind(GL_FRAMEBUFFER, 0);
            }
            (self.bind_buffer)(GL_PIXEL_UNPACK_BUFFER, 0);
            if let Some(b) = self.bind_sampler {
                b(0, 0);
            }
            if let Some(pm) = self.polygon_mode {
                pm(0x0408, 0x1B02);
            }
            (self.color_mask)(1, 1, 1, 1);
            (self.pixel_storei)(0x0CF2, 0);
            (self.pixel_storei)(0x0CF4, 0);
            (self.pixel_storei)(0x0CF3, 0);
            (self.pixel_storei)(0x0CF5, 4);
        }
    }

    unsafe fn restore(&self, s: &Saved) {
        unsafe {
            while (self.get_error)() != 0 {}
            if let Some(bind) = self.bind_framebuffer {
                bind(GL_DRAW_FRAMEBUFFER, s.draw_fb as c_uint);
                bind(GL_READ_FRAMEBUFFER, s.read_fb as c_uint);
            }
            (self.use_program)(s.program as c_uint);
            (self.active_texture)(GL_TEXTURE0);
            (self.bind_texture)(GL_TEXTURE_2D, s.texture0 as c_uint);
            if let Some(b) = self.bind_sampler {
                b(0, s.sampler0 as c_uint);
            }
            (self.active_texture)(s.active_texture as c_uint);
            if let Some(b) = self.bind_vertex_array {
                b(s.vao as c_uint);
            }
            (self.bind_buffer)(GL_ARRAY_BUFFER, s.array_buffer as c_uint);
            (self.bind_buffer)(GL_ELEMENT_ARRAY_BUFFER, s.element_buffer as c_uint);
            (self.bind_buffer)(GL_PIXEL_UNPACK_BUFFER, s.unpack_buffer as c_uint);
            (self.blend_func_separate)(
                s.blend[0] as c_uint,
                s.blend[1] as c_uint,
                s.blend[2] as c_uint,
                s.blend[3] as c_uint,
            );
            (self.blend_equation_separate)(s.blend[4] as c_uint, s.blend[5] as c_uint);
            (self.scissor)(s.scissor[0], s.scissor[1], s.scissor[2], s.scissor[3]);
            (self.viewport)(s.viewport[0], s.viewport[1], s.viewport[2], s.viewport[3]);
            (self.color_mask)(s.color_mask[0], s.color_mask[1], s.color_mask[2], s.color_mask[3]);
            for (i, e) in [0x0CF5, 0x0CF2, 0x0CF4, 0x0CF3].into_iter().enumerate() {
                (self.pixel_storei)(e, s.unpack[i]);
            }
            if let Some(pm) = self.polygon_mode {
                pm(0x0408, s.polygon_mode[0] as c_uint);
            }
            for (i, c) in CAPS.into_iter().enumerate() {
                if s.caps[i] {
                    (self.enable)(c);
                } else {
                    (self.disable)(c);
                }
            }
            while (self.get_error)() != 0 {}
        }
    }
}

/// Read the game's frame (the back buffer of the default framebuffer, before the overlay draws into it) and save it
/// as a screenshot. Every piece of state it touches is put back.
unsafe fn grab(get: &dyn Fn(&CStr) -> Fp, w: u32, h: u32) {
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return;
    }
    let Some(raw) = (unsafe { Raw::load(get) }) else { return };
    let rp = get(c"glReadPixels");
    if rp.is_null() {
        return;
    }
    let read_pixels: unsafe extern "C" fn(c_int, c_int, c_int, c_int, c_uint, c_uint, *mut c_void) =
        unsafe { std::mem::transmute(rp) };
    let rb = get(c"glReadBuffer");
    let read_buffer: Option<unsafe extern "C" fn(c_uint)> =
        if rb.is_null() { None } else { Some(unsafe { std::mem::transmute(rb) }) };
    let mut px = vec![0u8; w as usize * h as usize * 4];
    unsafe {
        while (raw.get_error)() != 0 {}
        // GL_PIXEL_PACK_BUFFER_BINDING, GL_PACK_ALIGNMENT / ROW_LENGTH / SKIP_ROWS / SKIP_PIXELS, GL_READ_BUFFER.
        let pack_buf = raw.int(0x88ED);
        let pack: Vec<i32> = [0x0D05, 0x0D02, 0x0D03, 0x0D04].into_iter().map(|e| raw.int(e)).collect();
        let read_buf = raw.int(0x0C02);
        let read_fb = if raw.bind_framebuffer.is_some() { raw.int(0x8CAA) } else { 0 };
        (raw.bind_buffer)(0x88EB, 0);
        for (e, v) in [0x0D05, 0x0D02, 0x0D03, 0x0D04].into_iter().zip([1, 0, 0, 0]) {
            (raw.pixel_storei)(e, v);
        }
        if let Some(bind) = raw.bind_framebuffer {
            bind(GL_READ_FRAMEBUFFER, 0);
        }
        if let Some(rb) = read_buffer {
            rb(0x0405); // GL_BACK
        }
        read_pixels(0, 0, w as c_int, h as c_int, 0x1908, 0x1401, px.as_mut_ptr().cast());
        let failed = (raw.get_error)() != 0;
        // Put everything back.
        if let Some(rb) = read_buffer {
            rb(read_buf as c_uint);
        }
        if let Some(bind) = raw.bind_framebuffer {
            bind(GL_READ_FRAMEBUFFER, read_fb as c_uint);
        }
        for (e, v) in [0x0D05, 0x0D02, 0x0D03, 0x0D04].into_iter().zip(pack) {
            (raw.pixel_storei)(e, v);
        }
        (raw.bind_buffer)(0x88EB, pack_buf as c_uint);
        while (raw.get_error)() != 0 {}
        if failed {
            return;
        }
    }
    crate::shot::finish_gl(&mut px, w as usize, h as usize);
    crate::shot::save(px, w, h);
}

/// What one GL context needs to draw.
struct Surface {
    raw: Raw,
    painter: Painter,
}

thread_local! {
    static SURFACES: RefCell<HashMap<usize, Option<Surface>>> = RefCell::new(HashMap::new());
}

/// Draw the overlay into the current context's default framebuffer. `get` looks GL functions up.
unsafe fn draw(ctx: usize, get: &dyn Fn(&CStr) -> Fp, size: impl FnOnce() -> (u32, u32)) {
    state::sync_backends();
    let claim = state::claim(ctx);
    if claim == Claim::Other {
        return;
    }
    let (w, h) = size();
    if state::take_shot() {
        unsafe { grab(get, w, h) };
    }
    let frame = {
        let mut c = state::lock();
        c.tick();
        c.frame(w, h)
    };
    let Some(mut frame) = frame else { return };
    SURFACES.with(|m| {
        let mut m = m.borrow_mut();
        if claim == Claim::New {
            m.remove(&ctx);
        }
        let entry = m.entry(ctx).or_insert_with(|| {
            let raw = unsafe { Raw::load(get) }?;
            let saved = unsafe { raw.save() };
            let gl = unsafe { egui_glow::glow::Context::from_loader_function_cstr(|n| get(n)) };
            let painter = Painter::new(Arc::new(gl), "", None, false);
            unsafe { raw.restore(&saved) };
            match painter {
                Ok(painter) => Some(Surface { raw, painter }),
                Err(e) => {
                    crate::ipc::log(&format!("gl painter failed: {e}"));
                    None
                }
            }
        });
        let Some(s) = entry else { return };
        static FIRST: std::sync::Once = std::sync::Once::new();
        FIRST.call_once(|| crate::ipc::log("gl first frame"));
        unsafe {
            let saved = s.raw.save();
            s.raw.prepare();
            s.painter.paint_and_update_textures([w, h], frame.pixels_per_point, &frame.primitives, &mut frame.textures);
            s.raw.restore(&saved);
        }
    });
}

fn glx_get() -> impl Fn(&CStr) -> Fp {
    |n: &CStr| {
        let f = real::sym_in(c"glXGetProcAddressARB", &[c"libGL.so.1", c"libGLX.so.0"])
            .or_else(|| real::sym_in(c"glXGetProcAddress", &[c"libGL.so.1", c"libGLX.so.0"]));
        match f {
            Some(f) => {
                let f: unsafe extern "C" fn(*const c_char) -> Fp = unsafe { std::mem::transmute(f) };
                unsafe { f(n.as_ptr()) }
            }
            None => std::ptr::null(),
        }
    }
}

fn egl_get() -> impl Fn(&CStr) -> Fp {
    |n: &CStr| match real::sym_in(c"eglGetProcAddress", &[c"libEGL.so.1"]) {
        Some(f) => {
            let f: unsafe extern "C" fn(*const c_char) -> Fp = unsafe { std::mem::transmute(f) };
            unsafe { f(n.as_ptr()) }
        }
        None => std::ptr::null(),
    }
}

unsafe fn glx_swap(dpy: *mut c_void, drawable: c_ulong) {
    let Some(cur) = real::sym_in(c"glXGetCurrentContext", &[c"libGL.so.1", c"libGLX.so.0"]) else { return };
    let ctx = unsafe { std::mem::transmute::<usize, unsafe extern "C" fn() -> usize>(cur)() };
    if ctx == 0 {
        return;
    }
    let size = || {
        let q = real::sym_in(c"glXQueryDrawable", &[c"libGL.so.1", c"libGLX.so.0"]);
        let (mut w, mut h) = (0u32, 0u32);
        if let Some(q) = q {
            let q: unsafe extern "C" fn(*mut c_void, c_ulong, c_int, *mut c_uint) = unsafe { std::mem::transmute(q) };
            unsafe {
                q(dpy, drawable, 0x801D, &mut w);
                q(dpy, drawable, 0x801E, &mut h);
            }
        }
        (w, h)
    };
    unsafe { draw(ctx, &glx_get(), size) };
}

unsafe fn egl_swap(dpy: *mut c_void, surface: *mut c_void) {
    let Some(cur) = real::sym_in(c"eglGetCurrentContext", &[c"libEGL.so.1"]) else { return };
    let ctx = unsafe { std::mem::transmute::<usize, unsafe extern "C" fn() -> usize>(cur)() };
    if ctx == 0 {
        return;
    }
    let size = || {
        let q = real::sym_in(c"eglQuerySurface", &[c"libEGL.so.1"]);
        let (mut w, mut h) = (0i32, 0i32);
        if let Some(q) = q {
            let q: unsafe extern "C" fn(*mut c_void, *mut c_void, c_int, *mut c_int) -> c_uint =
                unsafe { std::mem::transmute(q) };
            unsafe {
                q(dpy, surface, 0x3057, &mut w);
                q(dpy, surface, 0x3056, &mut h);
            }
        }
        (w.max(0) as u32, h.max(0) as u32)
    };
    unsafe { draw(ctx, &egl_get(), size) };
}

/// What a lookup of `name` answers: ours for the swap functions, the real one for the rest.
unsafe fn lookup(name: *const c_char, real_ptr: Fp) -> Fp {
    if name.is_null() || real_ptr.is_null() {
        return real_ptr;
    }
    match real::find(unsafe { CStr::from_ptr(name) }) {
        Some(slot) if slot.name.starts_with("glX") || slot.name.starts_with("egl") => {
            slot.set_real(real_ptr as usize);
            slot.ours()
        }
        _ => real_ptr,
    }
}

hooks! {
    fn glXSwapBuffers(real; dpy: *mut c_void, drawable: c_ulong) -> () {
        unsafe { glx_swap(dpy, drawable); real(dpy, drawable) }
    }
    fn glXGetProcAddress(real; name: *const c_char) -> Fp {
        let p = unsafe { real(name) };
        unsafe { lookup(name, p) }
    }
    fn glXGetProcAddressARB(real; name: *const c_char) -> Fp {
        let p = unsafe { real(name) };
        unsafe { lookup(name, p) }
    }
    fn eglSwapBuffers(real; dpy: *mut c_void, surface: *mut c_void) -> c_uint {
        unsafe { egl_swap(dpy, surface); real(dpy, surface) }
    }
    fn eglSwapBuffersWithDamageEXT(real; dpy: *mut c_void, surface: *mut c_void, rects: *mut c_int, n: c_int) -> c_uint {
        unsafe { egl_swap(dpy, surface); real(dpy, surface, rects, n) }
    }
    fn eglSwapBuffersWithDamageKHR(real; dpy: *mut c_void, surface: *mut c_void, rects: *mut c_int, n: c_int) -> c_uint {
        unsafe { egl_swap(dpy, surface); real(dpy, surface, rects, n) }
    }
    fn eglGetProcAddress(real; name: *const c_char) -> Fp {
        let p = unsafe { real(name) };
        unsafe { lookup(name, p) }
    }
}
