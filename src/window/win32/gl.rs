//! The OpenGL/WGL half of the Windows window backend: the pixel format,
//! the WGL context, the GL function table, and the whole `gfx.*` draw-call
//! surface — composed over the Win32-generic window machinery in
//! [`super::shared`] (`Win32WindowState`), the exact structural twin of
//! `x11/gl.rs` over `x11/shared.rs`.
//!
//! **Linking strategy** (deliberate, and different from Linux's): every
//! library this backend needs — `user32`, `gdi32`, `opengl32` — ships on
//! every Windows install with no separate "dev package" step (unlike Linux,
//! where GL dev headers/libs vary a lot across distros), so there is no
//! `dlopen`/`dlsym` dance here: `gdi32`/`opengl32` are linked normally
//! (user32 lives in `shared.rs`), and
//! `wglCreateContext`/`wglMakeCurrent`/`wglDeleteContext` are declared as
//! plain `extern "system"` items against `opengl32.dll`, mirroring how the
//! Linux backend (`x11/shared.rs`) links `libX11` normally while only
//! `libGL` is resolved dynamically (the reasons for dynamic GL resolution
//! there don't apply on Windows: `opengl32.dll` is a guaranteed system
//! component).
//!
//! Struct layouts and function prototypes below are drawn from the frozen
//! Win32 ABI (`wingdi.h`, unrevised since Windows NT 3.1/95) — this
//! session's egress policy blocked a direct fetch of the MSDN/header
//! text, so field order/values were cross-corroborated via web search
//! against multiple independent sources rather than read from a single raw
//! header, the same caveat `x11/gl.rs`'s module docs note for its GLX
//! tokens.

use std::ffi::{c_char, c_void, CString};
use std::ptr;

use super::shared::{GetDC, ReleaseDC, Win32WindowState, HDC};

// ---------------------------------------------------------------------------
// WGL types (wingdi.h). The windowing types (`HWND`, `HDC`, ...) live in
// `shared.rs`; `HGLRC` is WGL's own.
// ---------------------------------------------------------------------------

#[allow(clippy::upper_case_acronyms)]
type HGLRC = *mut c_void;

/// `wingdi.h`, 26 fields. `DWORD` is fixed 32-bit even on 64-bit Windows
/// (unlike POSIX `c_ulong`), so every `DWORD`/`WORD`/`BYTE` field below maps
/// to `u32`/`u16`/`u8` respectively, never a `c_long`-family type.
#[repr(C)]
struct PixelFormatDescriptor {
    n_size: u16,
    n_version: u16,
    dw_flags: u32,
    i_pixel_type: u8,
    c_color_bits: u8,
    c_red_bits: u8,
    c_red_shift: u8,
    c_green_bits: u8,
    c_green_shift: u8,
    c_blue_bits: u8,
    c_blue_shift: u8,
    c_alpha_bits: u8,
    c_alpha_shift: u8,
    c_accum_bits: u8,
    c_accum_red_bits: u8,
    c_accum_green_bits: u8,
    c_accum_blue_bits: u8,
    c_accum_alpha_bits: u8,
    c_depth_bits: u8,
    c_stencil_bits: u8,
    c_aux_buffers: u8,
    i_layer_type: u8,
    b_reserved: u8,
    dw_layer_mask: u32,
    dw_visible_mask: u32,
    dw_damage_mask: u32,
}

// wingdi.h constants (values as documented in the task brief).
const PFD_DRAW_TO_WINDOW: u32 = 0x0000_0004;
const PFD_SUPPORT_OPENGL: u32 = 0x0000_0020;
const PFD_DOUBLEBUFFER: u32 = 0x0000_0001;
const PFD_TYPE_RGBA: u8 = 0;
const PFD_MAIN_PLANE: u8 = 0;

const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;

// GL enum constants for the GL 3.3+ core-profile function-pointer table
// (`GlFns`, below) — values cross-corroborated against Khronos's
// `glcorearb.h` (see the task brief), same sourcing caveat as this module's
// other constants.
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

#[link(name = "gdi32")]
extern "system" {
    fn ChoosePixelFormat(hdc: HDC, ppfd: *const PixelFormatDescriptor) -> i32;
    fn SetPixelFormat(hdc: HDC, i_pixel_format: i32, ppfd: *const PixelFormatDescriptor) -> i32;
    fn SwapBuffers(hdc: HDC) -> i32;
}

#[link(name = "opengl32")]
extern "system" {
    fn wglCreateContext(hdc: HDC) -> HGLRC;
    fn wglMakeCurrent(hdc: HDC, hglrc: HGLRC) -> i32;
    fn wglDeleteContext(hglrc: HGLRC) -> i32;
    fn wglGetCurrentContext() -> HGLRC;
    /// Resolves any GL entry point past the static ABI floor `opengl32.dll`
    /// exports (GL 1.2, matching `libGL.so.1`'s guaranteed floor on Linux —
    /// see `x11/gl.rs`'s module doc comment) — only returns a valid pointer
    /// once a WGL context is current on this thread (see `GlFns::load`'s
    /// doc comment, below).
    fn wglGetProcAddress(lpsz_proc: *const c_char) -> *mut c_void;
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);

    // GL 1.0/1.1 entry points `opengl32.dll` statically exports, needed by
    // the GL 3.3+ core-profile draw path (`GlFns`, below) alongside the two
    // GL 1.0 calls above — no `wglGetProcAddress` resolution needed for
    // these, exactly like `glClearColor`/`glClear`.
    fn glTexImage2D(
        target: u32,
        level: i32,
        internalformat: i32,
        width: i32,
        height: i32,
        border: i32,
        format: u32,
        type_: u32,
        pixels: *const c_void,
    );
    fn glTexParameteri(target: u32, pname: u32, param: i32);
    fn glViewport(x: i32, y: i32, width: i32, height: i32);
    fn glEnable(cap: u32);
    fn glDisable(cap: u32);
    fn glGetError() -> u32;
    fn glGenTextures(n: i32, textures: *mut u32);
    fn glBindTexture(target: u32, texture: u32);
    fn glDeleteTextures(n: i32, textures: *const u32);
    fn glDrawArrays(mode: u32, first: i32, count: i32);
    fn glDrawElements(mode: u32, count: i32, type_: u32, indices: *const c_void);
    fn glReadPixels(
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        format: u32,
        type_: u32,
        pixels: *mut c_void,
    );
}

// ---------------------------------------------------------------------------
// GL 3.3+ core-profile function-pointer table (`GlFns`) — every entry point
// is `unsafe extern "system"` (not `"C"`): GL entry points on Windows use
// `APIENTRY`/`__stdcall`, matching the `glClearColor`/`glClear` items in the
// `#[link(name = "opengl32")]` block above.
// ---------------------------------------------------------------------------

type FnClearColor = unsafe extern "system" fn(f32, f32, f32, f32);
type FnClear = unsafe extern "system" fn(u32);

// -- direct-link (opengl32.dll static exports; GL 1.0-1.2 ABI floor) --
type FnTexImage2D =
    unsafe extern "system" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void);
type FnTexParameterI = unsafe extern "system" fn(u32, u32, i32);
type FnViewport = unsafe extern "system" fn(i32, i32, i32, i32);
type FnEnable = unsafe extern "system" fn(u32);
type FnDisable = unsafe extern "system" fn(u32);
type FnGetError = unsafe extern "system" fn() -> u32;
type FnGenTextures = unsafe extern "system" fn(i32, *mut u32);
type FnBindTexture = unsafe extern "system" fn(u32, u32);
type FnDeleteTextures = unsafe extern "system" fn(i32, *const u32);
type FnDrawArrays = unsafe extern "system" fn(u32, i32, i32);
type FnDrawElements = unsafe extern "system" fn(u32, i32, u32, *const c_void);
type FnReadPixels = unsafe extern "system" fn(i32, i32, i32, i32, u32, u32, *mut c_void);

// -- proc-address (wglGetProcAddress, resolved per-`Window` — see
// `GlFns::load`) --
type FnCreateShader = unsafe extern "system" fn(u32) -> u32;
type FnShaderSource = unsafe extern "system" fn(u32, i32, *const *const c_char, *const i32);
type FnCompileShader = unsafe extern "system" fn(u32);
type FnGetShaderIv = unsafe extern "system" fn(u32, u32, *mut i32);
type FnGetShaderInfoLog = unsafe extern "system" fn(u32, i32, *mut i32, *mut c_char);
type FnDeleteShader = unsafe extern "system" fn(u32);

type FnCreateProgram = unsafe extern "system" fn() -> u32;
type FnAttachShader = unsafe extern "system" fn(u32, u32);
type FnLinkProgram = unsafe extern "system" fn(u32);
type FnGetProgramIv = unsafe extern "system" fn(u32, u32, *mut i32);
type FnGetProgramInfoLog = unsafe extern "system" fn(u32, i32, *mut i32, *mut c_char);
type FnUseProgram = unsafe extern "system" fn(u32);
type FnDeleteProgram = unsafe extern "system" fn(u32);

type FnGenBuffers = unsafe extern "system" fn(i32, *mut u32);
type FnBindBuffer = unsafe extern "system" fn(u32, u32);
type FnBufferData = unsafe extern "system" fn(u32, isize, *const c_void, u32);
type FnDeleteBuffers = unsafe extern "system" fn(i32, *const u32);

type FnGenVertexArrays = unsafe extern "system" fn(i32, *mut u32);
type FnBindVertexArray = unsafe extern "system" fn(u32);
type FnDeleteVertexArrays = unsafe extern "system" fn(i32, *const u32);
type FnVertexAttribPointer = unsafe extern "system" fn(u32, i32, u32, u8, i32, *const c_void);
type FnEnableVertexAttribArray = unsafe extern "system" fn(u32);
type FnDisableVertexAttribArray = unsafe extern "system" fn(u32);

type FnActiveTexture = unsafe extern "system" fn(u32);

type FnGetUniformLocation = unsafe extern "system" fn(u32, *const c_char) -> i32;
type FnUniform1I = unsafe extern "system" fn(i32, i32);
type FnUniform1F = unsafe extern "system" fn(i32, f32);
type FnUniform2F = unsafe extern "system" fn(i32, f32, f32);
type FnUniform3F = unsafe extern "system" fn(i32, f32, f32, f32);
type FnUniform4F = unsafe extern "system" fn(i32, f32, f32, f32, f32);
type FnUniformMatrix4Fv = unsafe extern "system" fn(i32, i32, u8, *const f32);

/// The GL function table this namespace calls — the same GLEW-equivalent
/// loader concept as `x11/gl.rs`'s `GlFns`, extended here from the original two
/// GL 1.0 draw calls (`clear_color`/`clear`) to the full GL 3.3+
/// core-profile table `gfx.*` (a later PR) needs. Two families of field,
/// resolved differently:
///
/// - **Direct-link** (`clear_color`/`clear` plus the twelve GL 1.0–1.2
///   fields below them): statically exported by `opengl32.dll` — every
///   Windows install's guaranteed GL ABI floor, the same guarantee
///   `libGL.so.1` makes on Linux (see `x11/gl.rs`'s module doc comment).
///   Declared as plain `extern "system"` items in the
///   `#[link(name = "opengl32")]` block above; no runtime resolution is
///   needed, so `GlFns::load` just assigns each linked item straight into
///   its field.
/// - **Proc-address** (everything else, *including* `active_texture` — GL
///   1.3, one minor version past the static floor, so it does NOT get a
///   free ride despite looking "old"): resolved with `wglGetProcAddress`.
///   Unlike `x11/gl.rs`'s `glXGetProcAddress` (context-independent, safely
///   cached process-wide in a `OnceLock`) or this file's sibling platforms'
///   plain `dlsym`, `wglGetProcAddress` only returns a valid pointer once a
///   WGL context is current *on the calling thread*, and MSDN documents it
///   as potentially returning different addresses for different pixel
///   formats/contexts. So `GlFns::load` is deliberately **not**
///   `OnceLock`-cached here — it runs once per `Window`, from
///   `Inner::create`, immediately after that window's `wglMakeCurrent`
///   first succeeds, never memoized process-wide.
///
/// Plain function pointers, `Copy`: nothing here owns a resource a `Drop`
/// would need to release, exactly like `x11/gl.rs`/`macos/gl.rs`'s `GlFns`.
#[derive(Clone, Copy)]
struct GlFns {
    clear_color: FnClearColor,
    clear: FnClear,

    tex_image_2d: FnTexImage2D,
    tex_parameter_i: FnTexParameterI,
    viewport: FnViewport,
    enable: FnEnable,
    disable: FnDisable,
    // Resolved but not yet called anywhere: no `gfx.*` member currently
    // surfaces raw GL error state to Socrates. Reserved for a fuller `gfx` API,
    // matching `x11/gl.rs`'s identical note.
    #[allow(dead_code)]
    get_error: FnGetError,
    gen_textures: FnGenTextures,
    bind_texture: FnBindTexture,
    delete_textures: FnDeleteTextures,
    draw_arrays: FnDrawArrays,
    draw_elements: FnDrawElements,
    read_pixels: FnReadPixels,

    create_shader: FnCreateShader,
    shader_source: FnShaderSource,
    compile_shader: FnCompileShader,
    get_shader_iv: FnGetShaderIv,
    get_shader_info_log: FnGetShaderInfoLog,
    delete_shader: FnDeleteShader,

    create_program: FnCreateProgram,
    attach_shader: FnAttachShader,
    link_program: FnLinkProgram,
    get_program_iv: FnGetProgramIv,
    get_program_info_log: FnGetProgramInfoLog,
    use_program: FnUseProgram,
    delete_program: FnDeleteProgram,

    gen_buffers: FnGenBuffers,
    bind_buffer: FnBindBuffer,
    buffer_data: FnBufferData,
    delete_buffers: FnDeleteBuffers,

    gen_vertex_arrays: FnGenVertexArrays,
    bind_vertex_array: FnBindVertexArray,
    delete_vertex_arrays: FnDeleteVertexArrays,
    vertex_attrib_pointer: FnVertexAttribPointer,
    enable_vertex_attrib_array: FnEnableVertexAttribArray,
    disable_vertex_attrib_array: FnDisableVertexAttribArray,

    active_texture: FnActiveTexture,

    get_uniform_location: FnGetUniformLocation,
    uniform_1i: FnUniform1I,
    uniform_1f: FnUniform1F,
    uniform_2f: FnUniform2F,
    uniform_3f: FnUniform3F,
    uniform_4f: FnUniform4F,
    uniform_matrix_4fv: FnUniformMatrix4Fv,
}

impl GlFns {
    /// Resolve every field above against *this thread's currently current*
    /// WGL context — see the struct doc comment for why this, unlike
    /// `x11/gl.rs`/`macos/gl.rs`'s `GlFns::load`, is neither `OnceLock`-cached nor
    /// safe to call before `wglMakeCurrent` has succeeded. Any failure
    /// (a symbol the driver doesn't expose) is a clean `Err`, matching this
    /// module's "no partial resource leaks on a fallible step" discipline —
    /// the caller (`Inner::create`) tears down the context/DC/window it
    /// already made before propagating it.
    fn load() -> Result<Self, String> {
        // Safety: every direct-link field is a plain same-signature
        // function-pointer assignment of an already-linked `extern
        // "system"` item declared in the `#[link(name = "opengl32")]` block
        // above — not FFI in itself. Every proc-address field is resolved
        // through `wglGetProcAddress`; its returned pointer is null-checked
        // before being `transmute`d to the exact signature declared for
        // that symbol (taken from Khronos's `glcorearb.h`, per the task
        // brief — passing the wrong signature to a real function pointer
        // is undefined behavior, so these must stay byte-for-byte correct).
        unsafe {
            macro_rules! proc_addr {
                ($name:literal, $ty:ty) => {{
                    let cname = CString::new($name).unwrap();
                    let p = wglGetProcAddress(cname.as_ptr());
                    // `wglGetProcAddress` is documented (MSDN, and reiterated
                    // in every serious GL loader's own source — GLEW, GLAD,
                    // GLFW) to return these four sentinel values instead of
                    // NULL on some drivers when the requested function isn't
                    // actually supported: `1`, `2`, `3`, or `-1` (i.e.
                    // `0xFFFFFFFF`/`0xFFFFFFFFFFFFFFFF` as a pointer). A
                    // null-only check would `transmute` one of these bogus
                    // addresses into a real function pointer, crashing the
                    // first time it's called rather than erroring cleanly
                    // here — `glXGetProcAddress` (Linux) and `dlsym` (macOS)
                    // don't have this failure mode, only this platform does.
                    let addr = p as usize;
                    if p.is_null() || matches!(addr, 1..=3) || addr == usize::MAX {
                        return Err(format!(
                            "window.create: wglGetProcAddress could not resolve `{}`",
                            $name
                        ));
                    }
                    std::mem::transmute::<*mut c_void, $ty>(p)
                }};
            }
            Ok(GlFns {
                clear_color: glClearColor,
                clear: glClear,

                tex_image_2d: glTexImage2D,
                tex_parameter_i: glTexParameteri,
                viewport: glViewport,
                enable: glEnable,
                disable: glDisable,
                get_error: glGetError,
                gen_textures: glGenTextures,
                bind_texture: glBindTexture,
                delete_textures: glDeleteTextures,
                draw_arrays: glDrawArrays,
                draw_elements: glDrawElements,
                read_pixels: glReadPixels,

                create_shader: proc_addr!("glCreateShader", FnCreateShader),
                shader_source: proc_addr!("glShaderSource", FnShaderSource),
                compile_shader: proc_addr!("glCompileShader", FnCompileShader),
                get_shader_iv: proc_addr!("glGetShaderiv", FnGetShaderIv),
                get_shader_info_log: proc_addr!("glGetShaderInfoLog", FnGetShaderInfoLog),
                delete_shader: proc_addr!("glDeleteShader", FnDeleteShader),

                create_program: proc_addr!("glCreateProgram", FnCreateProgram),
                attach_shader: proc_addr!("glAttachShader", FnAttachShader),
                link_program: proc_addr!("glLinkProgram", FnLinkProgram),
                get_program_iv: proc_addr!("glGetProgramiv", FnGetProgramIv),
                get_program_info_log: proc_addr!("glGetProgramInfoLog", FnGetProgramInfoLog),
                use_program: proc_addr!("glUseProgram", FnUseProgram),
                delete_program: proc_addr!("glDeleteProgram", FnDeleteProgram),

                gen_buffers: proc_addr!("glGenBuffers", FnGenBuffers),
                bind_buffer: proc_addr!("glBindBuffer", FnBindBuffer),
                buffer_data: proc_addr!("glBufferData", FnBufferData),
                delete_buffers: proc_addr!("glDeleteBuffers", FnDeleteBuffers),

                gen_vertex_arrays: proc_addr!("glGenVertexArrays", FnGenVertexArrays),
                bind_vertex_array: proc_addr!("glBindVertexArray", FnBindVertexArray),
                delete_vertex_arrays: proc_addr!("glDeleteVertexArrays", FnDeleteVertexArrays),
                vertex_attrib_pointer: proc_addr!("glVertexAttribPointer", FnVertexAttribPointer),
                enable_vertex_attrib_array: proc_addr!(
                    "glEnableVertexAttribArray",
                    FnEnableVertexAttribArray
                ),
                disable_vertex_attrib_array: proc_addr!(
                    "glDisableVertexAttribArray",
                    FnDisableVertexAttribArray
                ),

                active_texture: proc_addr!("glActiveTexture", FnActiveTexture),

                get_uniform_location: proc_addr!("glGetUniformLocation", FnGetUniformLocation),
                uniform_1i: proc_addr!("glUniform1i", FnUniform1I),
                uniform_1f: proc_addr!("glUniform1f", FnUniform1F),
                uniform_2f: proc_addr!("glUniform2f", FnUniform2F),
                uniform_3f: proc_addr!("glUniform3f", FnUniform3F),
                uniform_4f: proc_addr!("glUniform4f", FnUniform4F),
                uniform_matrix_4fv: proc_addr!("glUniformMatrix4fv", FnUniformMatrix4Fv),
            })
        }
    }
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
    (gl.shader_source)(shader, 1, &ptr, ptr::null());
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

/// The OpenGL half of a `WindowHandle` (see `super::Inner`'s enum): the
/// Win32-generic window state plus a current-capable WGL context.
pub struct Inner {
    win32: Win32WindowState,
    hdc: HDC,
    hglrc: HGLRC,
    gl: GlFns,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        // Safety: `create_window` (shared.rs) handles the class/window/state
        // recipe; everything after follows the standard minimal WGL
        // "pick+set a pixel format, create+make-current a GL context"
        // recipe. Every fallible step is checked and every resource created
        // before a later failure is torn down before returning `Err`
        // (`win32.teardown()` reclaims the window + state box), so no
        // partial window/DC/context leaks.
        unsafe {
            let win32 = Win32WindowState::create_window("window.create", title, w, h)?;

            let hdc = GetDC(win32.hwnd);
            if hdc.is_null() {
                win32.teardown();
                return Err("window.create: GetDC failed".to_string());
            }

            let mut pfd: PixelFormatDescriptor = std::mem::zeroed();
            pfd.n_size = std::mem::size_of::<PixelFormatDescriptor>() as u16;
            pfd.n_version = 1;
            pfd.dw_flags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
            pfd.i_pixel_type = PFD_TYPE_RGBA;
            pfd.c_color_bits = 32;
            pfd.c_depth_bits = 24;
            pfd.i_layer_type = PFD_MAIN_PLANE;

            let format = ChoosePixelFormat(hdc, &pfd);
            if format == 0 {
                ReleaseDC(win32.hwnd, hdc);
                win32.teardown();
                return Err("window.create: ChoosePixelFormat found no matching pixel format"
                    .to_string());
            }
            if SetPixelFormat(hdc, format, &pfd) == 0 {
                ReleaseDC(win32.hwnd, hdc);
                win32.teardown();
                return Err("window.create: SetPixelFormat failed".to_string());
            }

            let hglrc = wglCreateContext(hdc);
            if hglrc.is_null() {
                ReleaseDC(win32.hwnd, hdc);
                win32.teardown();
                return Err("window.create: wglCreateContext failed".to_string());
            }
            if wglMakeCurrent(hdc, hglrc) == 0 {
                wglDeleteContext(hglrc);
                ReleaseDC(win32.hwnd, hdc);
                win32.teardown();
                return Err("window.create: wglMakeCurrent failed".to_string());
            }

            // `GlFns::load` resolves the `wglGetProcAddress` fields against
            // *this* context, which is why it happens here — right after
            // `wglMakeCurrent` first succeeds — rather than being cached
            // process-wide (see `GlFns`'s doc comment). On failure, tear
            // down everything created so far, same discipline as every
            // other fallible step in this function.
            let gl = match GlFns::load() {
                Ok(gl) => gl,
                Err(e) => {
                    wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
                    wglDeleteContext(hglrc);
                    ReleaseDC(win32.hwnd, hdc);
                    win32.teardown();
                    return Err(e);
                }
            };

            win32.show();

            Ok(Inner {
                win32,
                hdc,
                hglrc,
                gl,
            })
        }
    }

    pub fn poll(&mut self) {
        self.win32.poll();
    }

    pub fn key_down(&self, name: &str) -> bool {
        self.win32.key_down(name)
    }

    // Method-shaped accessors over the shared window state — the uniform
    // surface `window/mod.rs`'s generic `WindowHandle` code calls across
    // all three platforms' backend enums.
    pub fn mouse(&self) -> (f64, f64) {
        self.win32.mouse
    }
    pub fn width(&self) -> i32 {
        self.win32.width
    }
    pub fn height(&self) -> i32 {
        self.win32.height
    }
    pub fn should_close(&self) -> bool {
        self.win32.should_close
    }
    /// This half is always OpenGL/WGL — `super::Inner`'s Vulkan variant
    /// reports its own name.
    pub fn backend_name(&self) -> &'static str {
        "opengl"
    }

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // Safety: makes this window's context current before issuing GL
        // calls — necessary if another `Window` made itself current since
        // this one was created (WGL contexts are current per-thread, not
        // per-window, exactly like GLX). `wglMakeCurrent` can fail (e.g. a
        // display-driver reset or an RDP disconnect/reconnect invalidates
        // existing WGL contexts) — skip the GL calls rather than issue them
        // with no context bound, which the GL spec leaves undefined.
        unsafe {
            if wglMakeCurrent(self.hdc, self.hglrc) != 0 {
                (self.gl.clear_color)(r, g, b, a);
                (self.gl.clear)(GL_COLOR_BUFFER_BIT);
            }
        }
    }

    pub fn swap_buffers(&mut self) {
        // Safety: same current-context caveat as `clear`.
        unsafe {
            if wglMakeCurrent(self.hdc, self.hglrc) != 0 {
                SwapBuffers(self.hdc);
            }
        }
    }

    // -----------------------------------------------------------------
    // gfx.* (v0.8) — GL 3.3 core-profile draw calls against this window's
    // context, consumed through `WindowHandle`'s `gl_*` wrappers
    // (`src/window/mod.rs`). Mirrors `x11/gl.rs`'s equivalent block exactly,
    // method-for-method — only the current-context call itself
    // (`wglMakeCurrent` vs. `glXMakeCurrent`) differs.
    // -----------------------------------------------------------------

    fn ensure_current(&mut self) -> bool {
        unsafe { wglMakeCurrent(self.hdc, self.hglrc) != 0 }
    }

    pub fn make_current(&mut self) {
        self.ensure_current();
    }

    /// `gfx.compile_program_spirv` exists for the Linux Vulkan backend's
    /// SPIR-V input; this backend takes GLSL source through
    /// `compile_program`, so this is a clean redirect.
    pub fn compile_program_spirv(&mut self, _vs: &[u8], _fs: &[u8]) -> Result<u32, String> {
        Err(
            "gfx.compile_program_spirv: the OpenGL backend takes GLSL source, not SPIR-V \
             binaries — use gfx.compile_program(vertex, fragment)"
                .to_string(),
        )
    }

    pub fn compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        if !self.ensure_current() {
            return Err("gfx: failed to make the GL context current".to_string());
        }
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
        if self.ensure_current() {
            unsafe { (self.gl.use_program)(program) };
        }
    }

    pub fn delete_program(&mut self, program: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.delete_program)(program) };
        }
    }

    pub fn create_buffer(&mut self) -> u32 {
        if !self.ensure_current() {
            return 0;
        }
        let mut name: u32 = 0;
        unsafe { (self.gl.gen_buffers)(1, &mut name) };
        name
    }

    pub fn delete_buffer(&mut self, buffer: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.delete_buffers)(1, &buffer) };
        }
    }

    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.bind_buffer)(gl_buffer_target(kind), buffer) };
        }
    }

    pub fn upload_buffer(
        &mut self,
        kind: crate::window::GfxBufferKind,
        data: &[u8],
        dynamic: bool,
    ) {
        if self.ensure_current() {
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
    }

    pub fn create_vertex_array(&mut self) -> u32 {
        if !self.ensure_current() {
            return 0;
        }
        let mut name: u32 = 0;
        unsafe { (self.gl.gen_vertex_arrays)(1, &mut name) };
        name
    }

    pub fn bind_vertex_array(&mut self, vao: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.bind_vertex_array)(vao) };
        }
    }

    pub fn delete_vertex_array(&mut self, vao: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.delete_vertex_arrays)(1, &vao) };
        }
    }

    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        if self.ensure_current() {
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
    }

    pub fn disable_vertex_attrib(&mut self, index: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.disable_vertex_attrib_array)(index) };
        }
    }

    pub fn create_texture(&mut self) -> u32 {
        if !self.ensure_current() {
            return 0;
        }
        let mut name: u32 = 0;
        unsafe { (self.gl.gen_textures)(1, &mut name) };
        name
    }

    pub fn delete_texture(&mut self, tex: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.delete_textures)(1, &tex) };
        }
    }

    pub fn bind_texture(&mut self, tex: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.bind_texture)(GL_TEXTURE_2D, tex) };
        }
    }

    pub fn active_texture_unit(&mut self, unit: u32) {
        if self.ensure_current() {
            unsafe { (self.gl.active_texture)(GL_TEXTURE0 + unit) };
        }
    }

    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        if self.ensure_current() {
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
                (self.gl.tex_parameter_i)(
                    GL_TEXTURE_2D,
                    GL_TEXTURE_WRAP_S,
                    GL_CLAMP_TO_EDGE as i32,
                );
                (self.gl.tex_parameter_i)(
                    GL_TEXTURE_2D,
                    GL_TEXTURE_WRAP_T,
                    GL_CLAMP_TO_EDGE as i32,
                );
            }
        }
    }

    fn uniform_location(&mut self, program: u32, name: &str) -> i32 {
        unsafe {
            (self.gl.use_program)(program);
            let cname = CString::new(name).unwrap_or_else(|_| CString::new("").unwrap());
            (self.gl.get_uniform_location)(program, cname.as_ptr())
        }
    }

    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        if self.ensure_current() {
            let loc = self.uniform_location(program, name);
            unsafe { (self.gl.uniform_1i)(loc, v) };
        }
    }

    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        if self.ensure_current() {
            let loc = self.uniform_location(program, name);
            unsafe { (self.gl.uniform_1f)(loc, v) };
        }
    }

    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        if self.ensure_current() {
            let loc = self.uniform_location(program, name);
            unsafe { (self.gl.uniform_2f)(loc, x, y) };
        }
    }

    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        if self.ensure_current() {
            let loc = self.uniform_location(program, name);
            unsafe { (self.gl.uniform_3f)(loc, x, y, z) };
        }
    }

    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        if self.ensure_current() {
            let loc = self.uniform_location(program, name);
            unsafe { (self.gl.uniform_4f)(loc, x, y, z, w) };
        }
    }

    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        if self.ensure_current() {
            let loc = self.uniform_location(program, name);
            unsafe { (self.gl.uniform_matrix_4fv)(loc, 1, GL_FALSE as u8, values.as_ptr()) };
        }
    }

    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        if self.ensure_current() {
            unsafe { (self.gl.draw_arrays)(GL_TRIANGLES, first, count) };
        }
    }

    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        if self.ensure_current() {
            unsafe {
                (self.gl.draw_elements)(
                    GL_TRIANGLES,
                    count,
                    GL_UNSIGNED_INT,
                    byte_offset as *const c_void,
                )
            };
        }
    }

    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        if self.ensure_current() {
            unsafe {
                (self.gl.clear_color)(r, g, b, a);
                (self.gl.clear)(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            }
        }
    }

    pub fn set_depth_test(&mut self, enabled: bool) {
        if self.ensure_current() {
            unsafe {
                if enabled {
                    (self.gl.enable)(GL_DEPTH_TEST);
                } else {
                    (self.gl.disable)(GL_DEPTH_TEST);
                }
            }
        }
    }

    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if self.ensure_current() {
            unsafe { (self.gl.viewport)(x, y, w, h) };
        }
    }

    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        let len = (w.max(0) as usize) * (h.max(0) as usize) * 4;
        let mut buf = vec![0u8; len];
        if len > 0 && self.ensure_current() {
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
    /// `Drop` (see the module docs on `src/window/mod.rs`). Order: release
    /// current (only if *this* context is the one bound on this thread —
    /// blindly releasing would break a second, still-live `Window`),
    /// destroy the GL context, release the device context, then hand the
    /// window + state box to `Win32WindowState::teardown` — the same
    /// reverse-creation order the pre-split single struct used.
    pub fn teardown(self) {
        // Safety: every handle here was produced by the matching WGL create
        // call in `Inner::create` and is torn down in the reverse order it
        // was created, exactly once (this method consumes `self` and
        // `self.win32`).
        unsafe {
            if wglGetCurrentContext() == self.hglrc {
                wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
            }
            wglDeleteContext(self.hglrc);
            ReleaseDC(self.win32.hwnd, self.hdc);
        }
        self.win32.teardown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetErrorMode(u_mode: u32) -> u32;
    }
    /// Suppresses the Windows Error Reporting "<program> has stopped
    /// working" dialog for the calling process. Without this, an unhandled
    /// SEH exception anywhere in this file's raw FFI (a bad struct layout, a
    /// wrong calling convention, an invalid transmuted function pointer)
    /// shows a modal dialog instead of terminating — on a headless CI runner
    /// nothing can ever click it, so the process hangs forever with no
    /// output instead of failing fast with a visible error.
    ///
    /// This was originally added as the leading theory for the
    /// `window feature (Win32/WGL)` job hanging indefinitely on every run.
    /// It turned out to be the wrong subsystem: job-log evidence showed all
    /// 79 lib unit tests, including this file's own
    /// `create_clear_swap_poll_close` below, pass in 0.04s — the real hang
    /// was in `tests/lsp_smoke.rs`'s `diagnostics_hover_definition` (a URI-
    /// escaping bug in that test's own harness, since fixed). Left in place
    /// anyway since it's a reasonable defensive measure regardless — it
    /// just wasn't "the fix" for that bug. Scoped to `#[cfg(test)]` since
    /// this is a CI-diagnostic measure, not a production behavior change
    /// for real Socrates programs.
    const SEM_FAILCRITICALERRORS: u32 = 0x0001;
    const SEM_NOGPFAULTERRORBOX: u32 = 0x0002;

    /// End-to-end smoke test: create a window, clear it, swap buffers, pump
    /// events, confirm it isn't asking to close, then tear it down. Skips
    /// gracefully (doesn't panic the suite) if window creation fails for any
    /// environment-specific reason (e.g. no display session), matching
    /// `x11/gl.rs`'s test's graceful-skip style. On `windows-latest` in CI —
    /// a real Windows machine, unlike this module's own author's Linux dev
    /// environment — this exercises the whole pipe for real.
    #[test]
    fn create_clear_swap_poll_close() {
        // See `SetErrorMode`'s doc comment above: without this, a crash
        // anywhere below shows an unclickable WER dialog on CI instead of
        // failing fast.
        unsafe { SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX) };
        let inner = match Inner::create("socrates window test", 320, 240) {
            Ok(inner) => inner,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };
        let mut inner = inner;
        assert_eq!(inner.width(), 320);
        assert_eq!(inner.height(), 240);
        inner.clear(0.1, 0.2, 0.3, 1.0);
        inner.swap_buffers();
        inner.poll();
        assert!(!inner.should_close());
        inner.teardown();
    }
}
