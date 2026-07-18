//! OpenGL/CGL backend for macOS: an `NSOpenGLContext` bound to a
//! [`super::shared::CocoaWindowState`]'s window, plus the GL 3.3
//! core-profile function table the backend-neutral `gfx` draw-call
//! namespace consumes.
//!
//! **GL resolution**: `NSOpenGLContext`/`NSOpenGLPixelFormat` (the classes
//! used to create/bind the context) are AppKit classes messaged through
//! Cocoa, not linked GL symbols; only the plain `gl*` draw calls need to be
//! resolved from the framework, via `dlopen("/System/Library/Frameworks/
//! OpenGL.framework/OpenGL")` + `dlsym` — the same `dlopen`/`dlsym` strategy
//! `x11/gl.rs` uses for `libGL.so.1` (Apple's `OpenGL.framework` has no stable
//! dev-symlink story either).
//!
//! `GlFns` carries a GL 3.3 core-profile function table (shaders, programs,
//! buffers, VAOs, textures, uniforms, draw calls) beyond the
//! `glClearColor`/`glClear` pair — mirrors `x11/gl.rs`'s `GlFns` exactly (same
//! field names/signatures, cross-corroborated against Khronos's own
//! `glcorearb.h` and the Linux OpenGL ABI spec), but simpler to resolve:
//! unlike `libGL.so.1`'s GL-1.2 static-export floor (`x11/gl.rs` needs
//! `glXGetProcAddress` for anything newer) or `opengl32.dll`'s equivalent
//! floor (`win32.rs` needs `wglGetProcAddress`), Apple's `OpenGL.framework`
//! exports every core-profile entry point this table needs directly, so all
//! 43 symbols resolve with the same plain `dlsym` the original two
//! (`glClearColor`/`glClear`) already used — no proc-address mechanism
//! exists on this platform at all.

use super::shared::CocoaWindowState;
use crate::objc::{class, objc_msgSend, sel, send0, send0_void, send1_obj, Object, SEL};
use std::ffi::{c_char, c_void, CString};
use std::os::raw::c_int;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Constants (cross-corroborated per the research brief; stable, unrevised
// since these APIs' introduction).
// ---------------------------------------------------------------------------

const NS_OPENGL_PFA_DOUBLE_BUFFER: u32 = 5;
const NS_OPENGL_PFA_COLOR_SIZE: u32 = 8;
const NS_OPENGL_PFA_DEPTH_SIZE: u32 = 12;

const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;

// GL 3.3-core enum tokens (gl.h / glcorearb.h; stable, unrevised values
// cross-corroborated against Khronos's own `glcorearb.h`) needed by the
// function table below and its `gfx` namespace callers — identical values
// to `x11/gl.rs`'s copy of this same block.
const GL_FALSE: u32 = 0x0000_0000;
// GL_TRUE/GL_NO_ERROR/GL_UNPACK_ALIGNMENT/GL_NEAREST/GL_REPEAT are part of
// the contracted GL 3.3-core token set but have no call site in the current
// `gfx` v1 surface — reserved for a fuller `gfx` API, matching `x11/gl.rs`'s
// identical note.
#[allow(dead_code)]
const GL_TRUE: u32 = 0x0000_0001;
#[allow(dead_code)]
const GL_NO_ERROR: u32 = 0x0000_0000;
const GL_DEPTH_BUFFER_BIT: u32 = 0x0000_0100;
const GL_TRIANGLES: u32 = 0x0000_0004;
const GL_DEPTH_TEST: u32 = 0x0000_0B71;
#[allow(dead_code)]
const GL_UNPACK_ALIGNMENT: u32 = 0x0000_0CF5;
const GL_TEXTURE_2D: u32 = 0x0000_0DE1;
const GL_UNSIGNED_BYTE: u32 = 0x0000_1401;
const GL_UNSIGNED_INT: u32 = 0x0000_1405;
const GL_FLOAT: u32 = 0x0000_1406;
const GL_RGB: u32 = 0x0000_1907;
const GL_RGBA: u32 = 0x0000_1908;
#[allow(dead_code)]
const GL_NEAREST: u32 = 0x0000_2600;
const GL_LINEAR: u32 = 0x0000_2601;
const GL_TEXTURE_MAG_FILTER: u32 = 0x0000_2800;
const GL_TEXTURE_MIN_FILTER: u32 = 0x0000_2801;
const GL_TEXTURE_WRAP_S: u32 = 0x0000_2802;
const GL_TEXTURE_WRAP_T: u32 = 0x0000_2803;
#[allow(dead_code)]
const GL_REPEAT: u32 = 0x0000_2901;
const GL_CLAMP_TO_EDGE: u32 = 0x0000_812F;
const GL_TEXTURE0: u32 = 0x0000_84C0;
const GL_ARRAY_BUFFER: u32 = 0x0000_8892;
const GL_ELEMENT_ARRAY_BUFFER: u32 = 0x0000_8893;
const GL_STATIC_DRAW: u32 = 0x0000_88E4;
const GL_DYNAMIC_DRAW: u32 = 0x0000_88E8;
const GL_FRAGMENT_SHADER: u32 = 0x0000_8B30;
const GL_VERTEX_SHADER: u32 = 0x0000_8B31;
const GL_COMPILE_STATUS: u32 = 0x0000_8B81;
const GL_LINK_STATUS: u32 = 0x0000_8B82;
const GL_INFO_LOG_LENGTH: u32 = 0x0000_8B84;

// ---------------------------------------------------------------------------
// GL function pointers, resolved at runtime via dlopen/dlsym — mirrors
// `x11/gl.rs`'s `GlFns` exactly, just against `OpenGL.framework` instead of
// `libGL.so.1`, and all resolved the same single way (see the module doc
// comment: no proc-address split is needed on this platform). The GL
// context itself is created/bound through Cocoa (`NSOpenGLContext`), not
// through this framework handle.
// ---------------------------------------------------------------------------

#[link(name = "dl")]
extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: c_int = 2;

type FnClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
type FnClear = unsafe extern "C" fn(u32);

// GL 3.3 core-profile function pointer types, resolved at runtime via
// `dlsym` against `OpenGL.framework` — every one of them, unlike
// `x11/gl.rs`/`win32.rs` which split into a direct-link subset and a
// proc-address subset (see the module doc comment). GLenum/GLuint/GLint/
// GLsizei map to `u32`/`i32`, GLboolean to `u8`, GLfloat to `f32`, and
// GLsizeiptr to `isize` (pointer-width).

// SHADERS
type FnCreateShader = unsafe extern "C" fn(u32) -> u32;
type FnShaderSource = unsafe extern "C" fn(u32, i32, *const *const c_char, *const i32);
type FnCompileShader = unsafe extern "C" fn(u32);
type FnGetShaderiv = unsafe extern "C" fn(u32, u32, *mut i32);
type FnGetShaderInfoLog = unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char);
type FnDeleteShader = unsafe extern "C" fn(u32);

// PROGRAMS
type FnCreateProgram = unsafe extern "C" fn() -> u32;
type FnAttachShader = unsafe extern "C" fn(u32, u32);
type FnLinkProgram = unsafe extern "C" fn(u32);
type FnGetProgramiv = unsafe extern "C" fn(u32, u32, *mut i32);
type FnGetProgramInfoLog = unsafe extern "C" fn(u32, i32, *mut i32, *mut c_char);
type FnUseProgram = unsafe extern "C" fn(u32);
type FnDeleteProgram = unsafe extern "C" fn(u32);

// BUFFERS
type FnGenBuffers = unsafe extern "C" fn(i32, *mut u32);
type FnBindBuffer = unsafe extern "C" fn(u32, u32);
type FnBufferData = unsafe extern "C" fn(u32, isize, *const c_void, u32);
type FnDeleteBuffers = unsafe extern "C" fn(i32, *const u32);

// VAO
type FnGenVertexArrays = unsafe extern "C" fn(i32, *mut u32);
type FnBindVertexArray = unsafe extern "C" fn(u32);
type FnDeleteVertexArrays = unsafe extern "C" fn(i32, *const u32);
type FnVertexAttribPointer = unsafe extern "C" fn(u32, i32, u32, u8, i32, *const c_void);
type FnEnableVertexAttribArray = unsafe extern "C" fn(u32);
type FnDisableVertexAttribArray = unsafe extern "C" fn(u32);

// TEXTURES
type FnGenTextures = unsafe extern "C" fn(i32, *mut u32);
type FnBindTexture = unsafe extern "C" fn(u32, u32);
type FnTexImage2D = unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
type FnTexParameteri = unsafe extern "C" fn(u32, u32, i32);
type FnActiveTexture = unsafe extern "C" fn(u32);
type FnDeleteTextures = unsafe extern "C" fn(i32, *const u32);

// UNIFORMS
type FnGetUniformLocation = unsafe extern "C" fn(u32, *const c_char) -> i32;
type FnUniform1i = unsafe extern "C" fn(i32, i32);
type FnUniform1f = unsafe extern "C" fn(i32, f32);
type FnUniform2f = unsafe extern "C" fn(i32, f32, f32);
type FnUniform3f = unsafe extern "C" fn(i32, f32, f32, f32);
type FnUniform4f = unsafe extern "C" fn(i32, f32, f32, f32, f32);
type FnUniformMatrix4fv = unsafe extern "C" fn(i32, i32, u8, *const f32);

// DRAW/MISC
type FnDrawArrays = unsafe extern "C" fn(u32, i32, i32);
type FnDrawElements = unsafe extern "C" fn(u32, i32, u32, *const c_void);
type FnViewport = unsafe extern "C" fn(i32, i32, i32, i32);
type FnEnable = unsafe extern "C" fn(u32);
type FnDisable = unsafe extern "C" fn(u32);
type FnGetError = unsafe extern "C" fn() -> u32;
type FnReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);

/// The GL entry points this backend's window plumbing needs
/// (`clear_color`/`clear`), plus a GL 3.3 core-profile function table
/// (shaders/programs/buffers/VAOs/textures/uniforms/draw calls) for the
/// backend-neutral `gfx` draw-call namespace that consumes it — mirrors
/// `x11/gl.rs`'s `GlFns` field-for-field. Plain function pointers, `Copy`: the
/// underlying library load is process-wide and permanent (see
/// [`GlFns::load`]), so there is nothing here for a `Drop` to release.
#[derive(Clone, Copy)]
struct GlFns {
    clear_color: FnClearColor,
    clear: FnClear,

    // SHADERS
    create_shader: FnCreateShader,
    shader_source: FnShaderSource,
    compile_shader: FnCompileShader,
    get_shader_iv: FnGetShaderiv,
    get_shader_info_log: FnGetShaderInfoLog,
    delete_shader: FnDeleteShader,

    // PROGRAMS
    create_program: FnCreateProgram,
    attach_shader: FnAttachShader,
    link_program: FnLinkProgram,
    get_program_iv: FnGetProgramiv,
    get_program_info_log: FnGetProgramInfoLog,
    use_program: FnUseProgram,
    delete_program: FnDeleteProgram,

    // BUFFERS
    gen_buffers: FnGenBuffers,
    bind_buffer: FnBindBuffer,
    buffer_data: FnBufferData,
    delete_buffers: FnDeleteBuffers,

    // VAO
    gen_vertex_arrays: FnGenVertexArrays,
    bind_vertex_array: FnBindVertexArray,
    delete_vertex_arrays: FnDeleteVertexArrays,
    vertex_attrib_pointer: FnVertexAttribPointer,
    enable_vertex_attrib_array: FnEnableVertexAttribArray,
    disable_vertex_attrib_array: FnDisableVertexAttribArray,

    // TEXTURES
    gen_textures: FnGenTextures,
    bind_texture: FnBindTexture,
    tex_image_2d: FnTexImage2D,
    tex_parameter_i: FnTexParameteri,
    active_texture: FnActiveTexture,
    delete_textures: FnDeleteTextures,

    // UNIFORMS
    get_uniform_location: FnGetUniformLocation,
    uniform_1i: FnUniform1i,
    uniform_1f: FnUniform1f,
    uniform_2f: FnUniform2f,
    uniform_3f: FnUniform3f,
    uniform_4f: FnUniform4f,
    uniform_matrix_4fv: FnUniformMatrix4fv,

    // DRAW/MISC
    draw_arrays: FnDrawArrays,
    draw_elements: FnDrawElements,
    viewport: FnViewport,
    enable: FnEnable,
    disable: FnDisable,
    // Resolved but not yet called anywhere: no `gfx.*` member currently
    // surfaces raw GL error state to Socrates. Reserved for a fuller `gfx` API,
    // matching `x11/gl.rs`'s identical note.
    #[allow(dead_code)]
    get_error: FnGetError,
    read_pixels: FnReadPixels,
}

impl GlFns {
    /// Loaded once per process, cached — same reasoning as `x11/gl.rs::GlFns`:
    /// never `dlclose`d, a broken driver reports the same cached `Err` on
    /// every subsequent attempt rather than retrying.
    fn load() -> Result<Self, String> {
        static CACHE: OnceLock<Result<GlFns, String>> = OnceLock::new();
        CACHE.get_or_init(Self::load_uncached).clone()
    }

    fn load_uncached() -> Result<GlFns, String> {
        // Safety: straightforward dlopen/dlsym FFI; every pointer handed in
        // is a valid NUL-terminated `CString`, every returned pointer is
        // null-checked before use.
        unsafe {
            let libname =
                CString::new("/System/Library/Frameworks/OpenGL.framework/OpenGL").unwrap();
            let lib = dlopen(libname.as_ptr(), RTLD_NOW);
            if lib.is_null() {
                return Err("window.create: dlopen(OpenGL.framework) failed".to_string());
            }
            macro_rules! sym {
                ($name:literal, $ty:ty) => {{
                    let cname = CString::new($name).unwrap();
                    let p = dlsym(lib, cname.as_ptr());
                    if p.is_null() {
                        return Err(format!(
                            "window.create: OpenGL.framework is missing the symbol `{}`",
                            $name
                        ));
                    }
                    std::mem::transmute::<*mut c_void, $ty>(p)
                }};
            }
            Ok(GlFns {
                clear_color: sym!("glClearColor", FnClearColor),
                clear: sym!("glClear", FnClear),

                // SHADERS
                create_shader: sym!("glCreateShader", FnCreateShader),
                shader_source: sym!("glShaderSource", FnShaderSource),
                compile_shader: sym!("glCompileShader", FnCompileShader),
                get_shader_iv: sym!("glGetShaderiv", FnGetShaderiv),
                get_shader_info_log: sym!("glGetShaderInfoLog", FnGetShaderInfoLog),
                delete_shader: sym!("glDeleteShader", FnDeleteShader),

                // PROGRAMS
                create_program: sym!("glCreateProgram", FnCreateProgram),
                attach_shader: sym!("glAttachShader", FnAttachShader),
                link_program: sym!("glLinkProgram", FnLinkProgram),
                get_program_iv: sym!("glGetProgramiv", FnGetProgramiv),
                get_program_info_log: sym!("glGetProgramInfoLog", FnGetProgramInfoLog),
                use_program: sym!("glUseProgram", FnUseProgram),
                delete_program: sym!("glDeleteProgram", FnDeleteProgram),

                // BUFFERS
                gen_buffers: sym!("glGenBuffers", FnGenBuffers),
                bind_buffer: sym!("glBindBuffer", FnBindBuffer),
                buffer_data: sym!("glBufferData", FnBufferData),
                delete_buffers: sym!("glDeleteBuffers", FnDeleteBuffers),

                // VAO
                gen_vertex_arrays: sym!("glGenVertexArrays", FnGenVertexArrays),
                bind_vertex_array: sym!("glBindVertexArray", FnBindVertexArray),
                delete_vertex_arrays: sym!("glDeleteVertexArrays", FnDeleteVertexArrays),
                vertex_attrib_pointer: sym!("glVertexAttribPointer", FnVertexAttribPointer),
                enable_vertex_attrib_array: sym!(
                    "glEnableVertexAttribArray",
                    FnEnableVertexAttribArray
                ),
                disable_vertex_attrib_array: sym!(
                    "glDisableVertexAttribArray",
                    FnDisableVertexAttribArray
                ),

                // TEXTURES
                gen_textures: sym!("glGenTextures", FnGenTextures),
                bind_texture: sym!("glBindTexture", FnBindTexture),
                tex_image_2d: sym!("glTexImage2D", FnTexImage2D),
                tex_parameter_i: sym!("glTexParameteri", FnTexParameteri),
                active_texture: sym!("glActiveTexture", FnActiveTexture),
                delete_textures: sym!("glDeleteTextures", FnDeleteTextures),

                // UNIFORMS
                get_uniform_location: sym!("glGetUniformLocation", FnGetUniformLocation),
                uniform_1i: sym!("glUniform1i", FnUniform1i),
                uniform_1f: sym!("glUniform1f", FnUniform1f),
                uniform_2f: sym!("glUniform2f", FnUniform2f),
                uniform_3f: sym!("glUniform3f", FnUniform3f),
                uniform_4f: sym!("glUniform4f", FnUniform4f),
                uniform_matrix_4fv: sym!("glUniformMatrix4fv", FnUniformMatrix4fv),

                // DRAW/MISC
                draw_arrays: sym!("glDrawArrays", FnDrawArrays),
                draw_elements: sym!("glDrawElements", FnDrawElements),
                viewport: sym!("glViewport", FnViewport),
                enable: sym!("glEnable", FnEnable),
                disable: sym!("glDisable", FnDisable),
                get_error: sym!("glGetError", FnGetError),
                read_pixels: sym!("glReadPixels", FnReadPixels),
            })
        }
    }
}

unsafe fn send_init_pixel_format(recv: *mut Object, s: SEL, attrs: *const u32) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, *const u32) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, attrs)
}
unsafe fn send_init_context(
    recv: *mut Object,
    s: SEL,
    format: *mut Object,
    share: *mut Object,
) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object, *mut Object) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, format, share)
}

/// `GfxBufferKind` (see `src/window/mod.rs`) → the GL enum for that binding
/// target, kept private to this file like every other raw GL constant.
fn gl_buffer_target(kind: crate::window::GfxBufferKind) -> u32 {
    match kind {
        crate::window::GfxBufferKind::Vertex => GL_ARRAY_BUFFER,
        crate::window::GfxBufferKind::Index => GL_ELEMENT_ARRAY_BUFFER,
    }
}

/// Fetch a GL info log sized exactly via a prior `GL_INFO_LOG_LENGTH` query
/// — never a guessed fixed buffer size. Mirrors `x11/gl.rs`'s helper of the
/// same name exactly.
unsafe fn fetch_info_log(log_len: i32, get_log: impl FnOnce(i32, *mut i32, *mut c_char)) -> String {
    if log_len <= 1 {
        return String::new();
    }
    let mut buf = vec![0u8; log_len as usize];
    let mut written: i32 = 0;
    get_log(log_len, &mut written, buf.as_mut_ptr() as *mut c_char);
    let n = (written.max(0) as usize).min(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// Compile one shader stage; `Err` carries the driver's compile log. Mirrors
/// `x11/gl.rs`'s helper of the same name exactly (`GlFns` is `Copy`, so this
/// needs no `&Inner` borrow).
unsafe fn compile_shader_stage(gl: &GlFns, kind: u32, src: &str) -> Result<u32, String> {
    let shader = (gl.create_shader)(kind);
    let csrc = CString::new(src).unwrap_or_else(|_| CString::new("").unwrap());
    let ptr = csrc.as_ptr();
    (gl.shader_source)(shader, 1, &ptr, std::ptr::null());
    (gl.compile_shader)(shader);
    let mut status: i32 = 0;
    (gl.get_shader_iv)(shader, GL_COMPILE_STATUS, &mut status);
    if status == 0 {
        let mut log_len: i32 = 0;
        (gl.get_shader_iv)(shader, GL_INFO_LOG_LENGTH, &mut log_len);
        let msg = fetch_info_log(log_len, |buf_len, out_len, buf| {
            (gl.get_shader_info_log)(shader, buf_len, out_len, buf)
        });
        (gl.delete_shader)(shader);
        return Err(msg);
    }
    Ok(shader)
}

/// The real guts of the OpenGL/CGL `WindowHandle` variant — a
/// [`CocoaWindowState`] plus a current `NSOpenGLContext`.
pub struct Inner {
    cocoa: CocoaWindowState,
    ctx: *mut Object,
    gl: GlFns,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        if !super::shared::is_main_thread() {
            return Err(
                "window.create: must run on the process's main thread (macOS requires all \
                 NSWindow/AppKit calls there); this fires from a `worker` isolate, or from \
                 any thread other than the one that started the program"
                    .to_string(),
            );
        }
        let gl = GlFns::load()?;
        let cocoa = CocoaWindowState::create_window(title, w, h)?;

        // Safety: every call below follows the standard minimal Cocoa
        // "create a GL-capable pixel format + context, bind to the window's
        // content view" recipe (the direct analog of `x11/gl.rs::create`'s GLX
        // recipe); every fallible step (a null return from `alloc`/
        // `init...`) is checked and anything already created is released
        // before returning `Err`.
        unsafe {
            // Pixel format attribute array: `{ ColorSize, 24, DepthSize, 24,
            // DoubleBuffer, 0 }` — boolean flags (DoubleBuffer) take no
            // following value, sized attributes do, `0` terminates. See the
            // module doc comment / research brief for the token table.
            let attrs: [u32; 6] = [
                NS_OPENGL_PFA_COLOR_SIZE,
                24,
                NS_OPENGL_PFA_DEPTH_SIZE,
                24,
                NS_OPENGL_PFA_DOUBLE_BUFFER,
                0,
            ];

            let fmt_class = class("NSOpenGLPixelFormat");
            let fmt_alloc = send0(fmt_class, sel("alloc"));
            if fmt_alloc.is_null() {
                cocoa.teardown();
                return Err("window.create: [NSOpenGLPixelFormat alloc] returned nil".to_string());
            }
            let fmt = send_init_pixel_format(fmt_alloc, sel("initWithAttributes:"), attrs.as_ptr());
            if fmt.is_null() {
                cocoa.teardown();
                return Err(
                    "window.create: NSOpenGLPixelFormat initWithAttributes: found no matching \
                     pixel format"
                        .to_string(),
                );
            }

            let ctx_class = class("NSOpenGLContext");
            let ctx_alloc = send0(ctx_class, sel("alloc"));
            if ctx_alloc.is_null() {
                send0_void(fmt, sel("release"));
                cocoa.teardown();
                return Err("window.create: [NSOpenGLContext alloc] returned nil".to_string());
            }
            let ctx = send_init_context(
                ctx_alloc,
                sel("initWithFormat:shareContext:"),
                fmt,
                std::ptr::null_mut(),
            );
            // `fmt` is only needed for this call — release right after,
            // mirroring `x11/gl.rs` freeing `XVisualInfo` right after
            // `glXCreateContext`.
            send0_void(fmt, sel("release"));
            if ctx.is_null() {
                cocoa.teardown();
                return Err(
                    "window.create: NSOpenGLContext initWithFormat:shareContext: returned nil"
                        .to_string(),
                );
            }

            let content_view = cocoa.content_view();
            send1_obj(ctx, sel("setView:"), content_view);
            send0_void(ctx, sel("makeCurrentContext"));

            cocoa.show();

            Ok(Inner { cocoa, ctx, gl })
        }
    }

    pub fn poll(&mut self) {
        self.cocoa.poll();
    }

    pub fn key_down(&self, name: &str) -> bool {
        self.cocoa.key_down(name)
    }

    pub fn mouse(&self) -> (f64, f64) {
        self.cocoa.mouse
    }

    pub fn width(&self) -> i32 {
        self.cocoa.width
    }

    pub fn height(&self) -> i32 {
        self.cocoa.height
    }

    pub fn should_close(&self) -> bool {
        self.cocoa.should_close
    }

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // Safety: makes this window's context current before issuing GL
        // calls — necessary if another `Window` made itself current since
        // this one was created (matches `x11/gl.rs::clear`'s same caveat for
        // GLX).
        unsafe {
            send0_void(self.ctx, sel("makeCurrentContext"));
            (self.gl.clear_color)(r, g, b, a);
            (self.gl.clear)(GL_COLOR_BUFFER_BIT);
        }
    }

    pub fn swap_buffers(&mut self) {
        // Safety: same current-context caveat as `clear`. `flushBuffer` is
        // `NSOpenGLContext`'s swap-buffers equivalent for a double-buffered
        // pixel format.
        unsafe {
            send0_void(self.ctx, sel("makeCurrentContext"));
            send0_void(self.ctx, sel("flushBuffer"));
        }
    }

    // -----------------------------------------------------------------
    // gfx.* (v0.8) — GL 3.3 core-profile draw calls against this window's
    // context, consumed through `WindowHandle`'s `gl_*` wrappers
    // (`src/window/mod.rs`). Mirrors `x11/gl.rs`'s equivalent block exactly,
    // method-for-method — only the current-context call itself
    // (`[ctx makeCurrentContext]` vs. `glXMakeCurrent`) differs, and it
    // never fails the way GLX/WGL's can (no bool return to check), so
    // `ensure_current` always reports success here.
    // -----------------------------------------------------------------

    fn ensure_current(&mut self) -> bool {
        unsafe { send0_void(self.ctx, sel("makeCurrentContext")) };
        true
    }

    pub fn make_current(&mut self) {
        self.ensure_current();
    }

    pub fn compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        self.ensure_current();
        let gl = self.gl;
        unsafe {
            let vs = compile_shader_stage(&gl, GL_VERTEX_SHADER, vertex_src)
                .map_err(|e| format!("vertex shader: {e}"))?;
            let fs = match compile_shader_stage(&gl, GL_FRAGMENT_SHADER, fragment_src) {
                Ok(fs) => fs,
                Err(e) => {
                    (gl.delete_shader)(vs);
                    return Err(format!("fragment shader: {e}"));
                }
            };
            let program = (gl.create_program)();
            (gl.attach_shader)(program, vs);
            (gl.attach_shader)(program, fs);
            (gl.link_program)(program);
            let mut status: i32 = 0;
            (gl.get_program_iv)(program, GL_LINK_STATUS, &mut status);
            if status == 0 {
                let mut log_len: i32 = 0;
                (gl.get_program_iv)(program, GL_INFO_LOG_LENGTH, &mut log_len);
                let msg = fetch_info_log(log_len, |buf_len, out_len, buf| {
                    (gl.get_program_info_log)(program, buf_len, out_len, buf)
                });
                (gl.delete_shader)(vs);
                (gl.delete_shader)(fs);
                (gl.delete_program)(program);
                return Err(format!("link: {msg}"));
            }
            (gl.delete_shader)(vs);
            (gl.delete_shader)(fs);
            Ok(program)
        }
    }

    pub fn use_program(&mut self, program: u32) {
        self.ensure_current();
        unsafe { (self.gl.use_program)(program) };
    }

    pub fn delete_program(&mut self, program: u32) {
        self.ensure_current();
        unsafe { (self.gl.delete_program)(program) };
    }

    pub fn create_buffer(&mut self) -> u32 {
        self.ensure_current();
        let mut name: u32 = 0;
        unsafe { (self.gl.gen_buffers)(1, &mut name) };
        name
    }

    pub fn delete_buffer(&mut self, buffer: u32) {
        self.ensure_current();
        unsafe { (self.gl.delete_buffers)(1, &buffer) };
    }

    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        self.ensure_current();
        unsafe { (self.gl.bind_buffer)(gl_buffer_target(kind), buffer) };
    }

    pub fn upload_buffer(
        &mut self,
        kind: crate::window::GfxBufferKind,
        data: &[u8],
        dynamic: bool,
    ) {
        self.ensure_current();
        let usage = if dynamic { GL_DYNAMIC_DRAW } else { GL_STATIC_DRAW };
        unsafe {
            (self.gl.buffer_data)(
                gl_buffer_target(kind),
                data.len() as isize,
                data.as_ptr() as *const c_void,
                usage,
            )
        };
    }

    pub fn create_vertex_array(&mut self) -> u32 {
        self.ensure_current();
        let mut name: u32 = 0;
        unsafe { (self.gl.gen_vertex_arrays)(1, &mut name) };
        name
    }

    pub fn bind_vertex_array(&mut self, vao: u32) {
        self.ensure_current();
        unsafe { (self.gl.bind_vertex_array)(vao) };
    }

    pub fn delete_vertex_array(&mut self, vao: u32) {
        self.ensure_current();
        unsafe { (self.gl.delete_vertex_arrays)(1, &vao) };
    }

    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        self.ensure_current();
        unsafe {
            (self.gl.vertex_attrib_pointer)(
                index,
                size,
                GL_FLOAT,
                GL_FALSE as u8,
                stride,
                offset as *const c_void,
            );
            (self.gl.enable_vertex_attrib_array)(index);
        }
    }

    pub fn disable_vertex_attrib(&mut self, index: u32) {
        self.ensure_current();
        unsafe { (self.gl.disable_vertex_attrib_array)(index) };
    }

    pub fn create_texture(&mut self) -> u32 {
        self.ensure_current();
        let mut name: u32 = 0;
        unsafe { (self.gl.gen_textures)(1, &mut name) };
        name
    }

    pub fn delete_texture(&mut self, tex: u32) {
        self.ensure_current();
        unsafe { (self.gl.delete_textures)(1, &tex) };
    }

    pub fn bind_texture(&mut self, tex: u32) {
        self.ensure_current();
        unsafe { (self.gl.bind_texture)(GL_TEXTURE_2D, tex) };
    }

    pub fn active_texture_unit(&mut self, unit: u32) {
        self.ensure_current();
        unsafe { (self.gl.active_texture)(GL_TEXTURE0 + unit) };
    }

    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        self.ensure_current();
        let format = if has_alpha { GL_RGBA } else { GL_RGB };
        unsafe {
            (self.gl.tex_image_2d)(
                GL_TEXTURE_2D,
                0,
                format as i32,
                width,
                height,
                0,
                format,
                GL_UNSIGNED_BYTE,
                data.as_ptr() as *const c_void,
            );
            (self.gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
            (self.gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as i32);
            (self.gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32);
            (self.gl.tex_parameter_i)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32);
        }
    }

    fn uniform_location(&mut self, program: u32, name: &str) -> i32 {
        self.ensure_current();
        unsafe {
            (self.gl.use_program)(program);
            let cname = CString::new(name).unwrap_or_else(|_| CString::new("").unwrap());
            (self.gl.get_uniform_location)(program, cname.as_ptr())
        }
    }

    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        let loc = self.uniform_location(program, name);
        unsafe { (self.gl.uniform_1i)(loc, v) };
    }

    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        let loc = self.uniform_location(program, name);
        unsafe { (self.gl.uniform_1f)(loc, v) };
    }

    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        let loc = self.uniform_location(program, name);
        unsafe { (self.gl.uniform_2f)(loc, x, y) };
    }

    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        let loc = self.uniform_location(program, name);
        unsafe { (self.gl.uniform_3f)(loc, x, y, z) };
    }

    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        let loc = self.uniform_location(program, name);
        unsafe { (self.gl.uniform_4f)(loc, x, y, z, w) };
    }

    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        let loc = self.uniform_location(program, name);
        unsafe { (self.gl.uniform_matrix_4fv)(loc, 1, GL_FALSE as u8, values.as_ptr()) };
    }

    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        self.ensure_current();
        unsafe { (self.gl.draw_arrays)(GL_TRIANGLES, first, count) };
    }

    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        self.ensure_current();
        unsafe {
            (self.gl.draw_elements)(GL_TRIANGLES, count, GL_UNSIGNED_INT, byte_offset as *const c_void)
        };
    }

    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.ensure_current();
        unsafe {
            (self.gl.clear_color)(r, g, b, a);
            (self.gl.clear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
    }

    pub fn set_depth_test(&mut self, enabled: bool) {
        self.ensure_current();
        unsafe {
            if enabled {
                (self.gl.enable)(GL_DEPTH_TEST);
            } else {
                (self.gl.disable)(GL_DEPTH_TEST);
            }
        }
    }

    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.ensure_current();
        unsafe { (self.gl.viewport)(x, y, w, h) };
    }

    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        let len = (w.max(0) as usize) * (h.max(0) as usize) * 4;
        let mut buf = vec![0u8; len];
        if len > 0 {
            self.ensure_current();
            unsafe {
                (self.gl.read_pixels)(
                    x,
                    y,
                    w,
                    h,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    buf.as_mut_ptr() as *mut c_void,
                );
            }
        }
        buf
    }

    /// Idempotent teardown, called by both `WindowHandle::close` and its
    /// `Drop` (see the module docs on `src/window/mod.rs`). Releases the GL
    /// context, then the window, in that order (reverse of creation) —
    /// matches `x11/gl.rs::teardown`'s ordering discipline. Does not release
    /// the process-lifetime `NSApplication`/autorelease pool/OpenGL
    /// framework handle, exactly as `x11/gl.rs` never closes its X `Display`'s
    /// underlying `libGL.so.1` `dlopen` handle — those are process-lifetime
    /// resources, not per-window ones.
    pub fn teardown(self) {
        // Safety: `ctx` was produced by a matching `alloc`+`init...` pair in
        // `Inner::create` and is still +1 owned (nothing else in this file
        // retains or releases it); releasing it here, then the window via
        // `CocoaWindowState::teardown`, is therefore balanced. `self` is
        // consumed so this can't run twice.
        unsafe { send0_void(self.ctx, sel("release")) };
        self.cocoa.teardown();
    }
}
