//! macOS (Apple Silicon / `aarch64-apple-darwin` only) Cocoa/AppKit/NSOpenGL
//! backend for the `window` namespace.
//!
//! **Why arm64-only**: x86_64 Macs require picking between `objc_msgSend` and
//! `objc_msgSend_stret` per call site depending on whether the returned
//! struct fits in registers — a second, easy-to-get-wrong dispatch path.
//! Apple's arm64 ABI returns small aggregates (including `NSRect`, the one
//! struct-returning message this file sends) directly in `x0`/`x1`/`v0`-`v3`,
//! and `objc_msgSend_stret` doesn't exist in the arm64 SDK at all — so plain
//! `objc_msgSend`, correctly `transmute`d per call shape, is the *only* path
//! and there is no split to get wrong. This matches the release matrix
//! (`aarch64-apple-darwin` is the only macOS target `fable build` staples
//! for), so nothing is lost by declining x86_64.
//!
//! **Linking strategy** (mirrors `x11.rs`'s doc comment):
//! - `Cocoa` (pulls in AppKit + Foundation transitively) is linked normally
//!   (`#[link(name = "Cocoa", kind = "framework")]`) — every Mac has it, it's
//!   part of the OS, unlike a GL *dev* package on Linux.
//! - `libobjc` (the Objective-C runtime: `objc_msgSend`, `objc_getClass`,
//!   `sel_registerName`) is linked normally too — also always present.
//! - GL itself is resolved dynamically via
//!   `dlopen("/System/Library/Frameworks/OpenGL.framework/OpenGL")` +
//!   `dlsym`, the same `dlopen`/`dlsym` strategy `x11.rs` uses for
//!   `libGL.so.1` — Apple's `OpenGL.framework` has no stable dev-symlink
//!   story either, and `NSOpenGLContext`/`NSOpenGLPixelFormat` (the classes
//!   actually used to create/bind the context) are AppKit classes messaged
//!   through Cocoa, not linked GL symbols; only the plain `gl*` draw calls
//!   need to be resolved from the framework.
//!
//! `GlFns` also carries a GL 3.3 core-profile function table (shaders,
//! programs, buffers, VAOs, textures, uniforms, draw calls) beyond the
//! `glClearColor`/`glClear` pair above, for the upcoming backend-neutral
//! `gfx` draw-call namespace — mirrors `x11.rs`'s `GlFns` exactly (same
//! field names/signatures, cross-corroborated against Khronos's own
//! `glcorearb.h` and the Linux OpenGL ABI spec), but simpler to resolve:
//! unlike `libGL.so.1`'s GL-1.2 static-export floor (`x11.rs` needs
//! `glXGetProcAddress` for anything newer) or `opengl32.dll`'s equivalent
//! floor (`win32.rs` needs `wglGetProcAddress`), Apple's `OpenGL.framework`
//! exports every core-profile entry point this table needs directly, so all
//! 43 new symbols resolve with the same plain `dlsym` the original two
//! (`glClearColor`/`glClear`) already used — no proc-address mechanism
//! exists on this platform at all.
//!
//! **`objc_msgSend` dispatch**: Rust has no variadic FFI, so there is no
//! single Rust signature for `objc_msgSend` — the raw symbol is declared
//! once below and `transmute`d to a distinct `unsafe extern "C" fn(...)`
//! type at every call site, one per distinct argument/return shape. This is
//! the standard, only-known-working pattern for calling the Objective-C
//! runtime from a non-Objective-C language (cross-corroborated against
//! cocoa-rs/objc-rs's own approach, since this session's egress to
//! docs.rs/GitHub raw was blocked the same way `x11.rs`'s GLX-token fetches
//! were — see that file's module doc comment for the precedent).
//!
//! **Memory management (no ARC in raw FFI)** — this file uses the coarse,
//! process-lifetime pattern rather than fine-grained manual retain/release
//! bookkeeping, deliberately (simplicity over precision, per the task brief
//! and in the same spirit as `x11.rs` never `dlclose`-ing `libGL.so.1`):
//! - One `NSAutoreleasePool` is created for the whole process (in
//!   `ensure_app_init`, once, via a `OnceLock`) and intentionally never
//!   drained/released. Every `alloc`-less convenience constructor used here
//!   (`[NSString stringWithUTF8String:]`, `[NSApplication sharedApplication]`,
//!   `[NSDate distantPast]`, the event objects `nextEventMatchingMask:...`
//!   hands back) is autoreleased into it. For a short-lived, single-threaded,
//!   poll-once-per-frame program that never wraps the loop in its own inner
//!   pool, autoreleased objects simply accumulate for the process's lifetime
//!   instead of being reclaimed promptly — acceptable here for the same
//!   reason `x11.rs` accepts `libGL.so.1` staying `dlopen`'d forever: bounded
//!   by process lifetime, not by anything this module loops over unboundedly
//!   per-frame (no autoreleased object is created more than a small constant
//!   number of times per `poll()` call).
//! - The four objects this module actually creates via `alloc`
//!   (`NSWindow`, `NSOpenGLPixelFormat`, `NSOpenGLContext`) are each a +1
//!   owned reference; `NSOpenGLPixelFormat` is released right after
//!   `initWithFormat:shareContext:` consumes it (mirrors `x11.rs` freeing
//!   `XVisualInfo` right after `glXCreateContext`), and `NSWindow` +
//!   `NSOpenGLContext` are released in `Inner::teardown`, in the reverse
//!   order they were created — the same discipline `x11.rs::teardown` uses
//!   for its X11/GLX handles.

use std::ffi::{c_char, c_void, CString};
use std::os::raw::c_int;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// Objective-C runtime primitives.
// ---------------------------------------------------------------------------

#[allow(clippy::upper_case_acronyms)] // matches Objective-C's own `SEL` name
type SEL = *mut c_void;
type Class = *mut Object;
#[repr(C)]
pub struct Object {
    _private: [u8; 0],
}

// `NSInteger`/`NSUInteger` are 64-bit on arm64; `BOOL` is a signed byte
// (`c_char`/`i8`) on Apple's modern ("NeXT") runtime, not `c_int` — a common
// mistake porting 32-bit-era Objective-C snippets.
type NsInteger = i64;
type NsUInteger = u64;
type ObjcBool = i8;
const OBJC_YES: ObjcBool = 1;
const OBJC_NO: ObjcBool = 0;

/// `CGFloat` is `f64` on every 64-bit Apple platform (arm64 included).
type CgFloat = f64;

#[repr(C)]
#[derive(Clone, Copy)]
struct NsPoint {
    x: CgFloat,
    y: CgFloat,
}
#[repr(C)]
#[derive(Clone, Copy)]
struct NsSize {
    width: CgFloat,
    height: CgFloat,
}
/// On arm64 this 32-byte-aggregate return needs no `objc_msgSend_stret`
/// special-casing (see the module doc comment) — only the correctly typed
/// `transmute` at the call site.
#[repr(C)]
#[derive(Clone, Copy)]
struct NsRect {
    origin: NsPoint,
    size: NsSize,
}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> SEL;
    /// Never called directly — every call site `transmute`s this to the
    /// exact argument/return shape it needs (Rust has no variadic FFI; see
    /// the module doc comment).
    fn objc_msgSend();
}

#[link(name = "Cocoa", kind = "framework")]
extern "C" {}

/// Resolve a class by name, panicking only if AppKit itself is missing the
/// class (would indicate a broken/ancient OS, not a recoverable condition —
/// `x11.rs` similarly treats a missing core X11 symbol as fatal via `dlsym`
/// returning null and an `Err`, but classes looked up by name with no
/// fallback path have no sensible `Result` to return through here since
/// every caller is inside `unsafe` setup code already committed to Cocoa
/// existing).
unsafe fn class(name: &str) -> Class {
    let cname = CString::new(name).unwrap();
    objc_getClass(cname.as_ptr())
}

unsafe fn sel(name: &str) -> SEL {
    let cname = CString::new(name).unwrap();
    sel_registerName(cname.as_ptr())
}

// Rather than one generic macro, every distinct `objc_msgSend` shape used in
// this file gets its own small named wrapper below — clearer to audit than a
// cleverer variadic-emulating macro, which matters more here than brevity
// given the risk profile (Objective-C selector/argument-encoding mistakes
// compile cleanly and only misbehave at runtime; see the module doc comment
// on `objc_msgSend` dispatch).
unsafe fn send0(recv: *mut Object, s: SEL) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
unsafe fn send0_bool(recv: *mut Object, s: SEL) -> ObjcBool {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> ObjcBool =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
unsafe fn send0_uint(recv: *mut Object, s: SEL) -> NsUInteger {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> NsUInteger =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
unsafe fn send0_ptr(recv: *mut Object, s: SEL) -> *const c_char {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> *const c_char =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
/// One-`NSInteger`-argument message that returns `BOOL` (e.g.
/// `setActivationPolicy:`) — its own wrapper rather than a `void`-returning
/// one with the result discarded, since discarding through a transmuted
/// function-pointer type that doesn't even declare the real return value is
/// itself a mistyped-function-pointer call (UB per Rust's FFI contract, even
/// though it happens to be harmless on arm64's calling convention for this
/// specific case).
unsafe fn send1_int_bool(recv: *mut Object, s: SEL, arg: NsInteger) -> ObjcBool {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsInteger) -> ObjcBool =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send0_void(recv: *mut Object, s: SEL) {
    let f: unsafe extern "C" fn(*mut Object, SEL) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
unsafe fn send1_obj(recv: *mut Object, s: SEL, arg: *mut Object) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send1_ptr_ret(recv: *mut Object, s: SEL, arg: *const c_char) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, *const c_char) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
/// One-`BOOL`-argument, `void`-returning message (e.g.
/// `activateIgnoringOtherApps:`).
unsafe fn send1_bool_void(recv: *mut Object, s: SEL, arg: ObjcBool) {
    let f: unsafe extern "C" fn(*mut Object, SEL, ObjcBool) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send_init_window(
    recv: *mut Object,
    s: SEL,
    rect: NsRect,
    style_mask: NsUInteger,
    backing: NsUInteger,
    defer: ObjcBool,
) -> *mut Object {
    let f: unsafe extern "C" fn(
        *mut Object,
        SEL,
        NsRect,
        NsUInteger,
        NsUInteger,
        ObjcBool,
    ) -> *mut Object = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, rect, style_mask, backing, defer)
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
#[allow(clippy::too_many_arguments)]
unsafe fn send_next_event(
    recv: *mut Object,
    s: SEL,
    mask: NsUInteger,
    until_date: *mut Object,
    mode: *mut Object,
    dequeue: ObjcBool,
) -> *mut Object {
    let f: unsafe extern "C" fn(
        *mut Object,
        SEL,
        NsUInteger,
        *mut Object,
        *mut Object,
        ObjcBool,
    ) -> *mut Object = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, mask, until_date, mode, dequeue)
}

// ---------------------------------------------------------------------------
// Constants (cross-corroborated per the research brief; stable, unrevised
// since these APIs' introduction).
// ---------------------------------------------------------------------------

const NS_APPLICATION_ACTIVATION_POLICY_REGULAR: NsInteger = 0;

const NS_WINDOW_STYLE_MASK_TITLED: NsUInteger = 1 << 0;
const NS_WINDOW_STYLE_MASK_CLOSABLE: NsUInteger = 1 << 1;
const NS_WINDOW_STYLE_MASK_MINIATURIZABLE: NsUInteger = 1 << 2;
const NS_WINDOW_STYLE_MASK_RESIZABLE: NsUInteger = 1 << 3;
const NS_BACKING_STORE_BUFFERED: NsUInteger = 2;

const NS_OPENGL_PFA_DOUBLE_BUFFER: u32 = 5;
const NS_OPENGL_PFA_COLOR_SIZE: u32 = 8;
const NS_OPENGL_PFA_DEPTH_SIZE: u32 = 12;

const NS_EVENT_TYPE_KEY_DOWN: NsUInteger = 10;
const NS_EVENT_TYPE_KEY_UP: NsUInteger = 11;
const NS_EVENT_TYPE_MOUSE_MOVED: NsUInteger = 5;
// Any-event mask for `nextEventMatchingMask:` — `NSUIntegerMax`, i.e. all
// bits set (64-bit on arm64).
const NS_ANY_EVENT_MASK: NsUInteger = u64::MAX;

const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;

// GL 3.3-core enum tokens (gl.h / glcorearb.h; stable, unrevised values
// cross-corroborated against Khronos's own `glcorearb.h`) needed by the
// function table below and its future `gfx` namespace callers — identical
// values to `x11.rs`'s copy of this same block.
const GL_FALSE: u32 = 0x0000_0000;
// GL_TRUE/GL_NO_ERROR/GL_UNPACK_ALIGNMENT/GL_NEAREST/GL_REPEAT are part of
// the contracted GL 3.3-core token set but have no call site in the current
// `gfx` v1 surface — reserved for a fuller `gfx` API, matching `x11.rs`'s
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
// `x11.rs`'s `GlFns` exactly, just against `OpenGL.framework` instead of
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
// `x11.rs`/`win32.rs` which split into a direct-link subset and a
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

/// The GL entry points this namespace's window plumbing needs
/// (`clear_color`/`clear`), plus a GL 3.3 core-profile function table
/// (shaders/programs/buffers/VAOs/textures/uniforms/draw calls) for the
/// backend-neutral `gfx` draw-call namespace that consumes it (a separate,
/// later PR — this struct and its loader are the GLEW-equivalent
/// groundwork, mirroring `x11.rs`'s `GlFns` field-for-field). Plain function
/// pointers, `Copy`: the underlying library load is process-wide and
/// permanent (see [`GlFns::load`]), so there is nothing here for a `Drop` to
/// release.
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
    // surfaces raw GL error state to Fable. Reserved for a fuller `gfx` API,
    // matching `x11.rs`'s identical note.
    #[allow(dead_code)]
    get_error: FnGetError,
    read_pixels: FnReadPixels,
}

impl GlFns {
    /// Loaded once per process, cached — same reasoning as `x11.rs::GlFns`:
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

// ---------------------------------------------------------------------------
// NSApplication one-time setup — must happen once per process before any
// window is created. Also where the process-lifetime autorelease pool
// (see the module doc comment) is created.
// ---------------------------------------------------------------------------

fn ensure_app_init() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| unsafe {
        // Process-lifetime autorelease pool — see the module doc comment on
        // memory management for why this is the deliberately coarse choice
        // here instead of per-frame pool push/pop.
        let pool_class = class("NSAutoreleasePool");
        let pool = send0(pool_class, sel("alloc"));
        // Intentionally never released/drained — a raw pointer has no
        // destructor to suppress, so simply not storing it anywhere is
        // enough to leak it for the process's lifetime (see the module doc
        // comment on memory management).
        send0(pool, sel("init"));

        let app_class = class("NSApplication");
        let app = send0(app_class, sel("sharedApplication"));
        // Return value (did the policy change take effect) intentionally
        // ignored — this mirrors x11.rs's "only check calls whose failure
        // blocks progress" convention; a regular-policy app that somehow
        // keeps its prior policy still creates and shows windows fine.
        let _ = send1_int_bool(
            app,
            sel("setActivationPolicy:"),
            NS_APPLICATION_ACTIVATION_POLICY_REGULAR,
        );

        // Deliberately *not* calling `[NSApp finishLaunching]`. Three
        // different attempts at giving `mainMenu` a value before that call
        // — nil (the original code), a bare item-less `NSMenu`, and a
        // GLFW-shaped skeleton (one empty-titled item with an empty
        // submenu standing in as the "Apple menu") — all three, verified on
        // real macos-14 CI hardware, hit the identical fatal assertion
        // (`-[NSMenu _setMenuName:]`, an unrecoverable SIGABRT, not a
        // catchable Objective-C exception) at the identical point, deep
        // inside `finishLaunching`'s own internal main-menu bootstrap. That
        // rules out "the menu's shape is wrong" as fixable from the outside
        // without access to AppKit's private implementation; the only
        // remaining lever is to not call the function that hosts it.
        //
        // `finishLaunching`'s other documented effects — posting
        // `NSApplicationWillFinishLaunchingNotification`/
        // `didFinishLaunching`, running an installed delegate's launch
        // hooks — don't apply here (this file installs no
        // `NSApplicationDelegate` and drives its own event loop directly in
        // `poll()`, mirroring `x11.rs`'s manual `XPending`/`XNextEvent`
        // pump rather than handing control to `[NSApp run]`). The one
        // remaining externally-visible effect worth keeping —
        // bringing the app/window to the front so it can become key and
        // receive keyboard events — is requested directly instead.
        send1_bool_void(app, sel("activateIgnoringOtherApps:"), OBJC_YES);
    });
}

/// `+[NSThread isMainThread]` — AppKit enforces, unconditionally, that
/// `NSWindow` (and UI objects generally) only ever get created on the
/// process's actual main thread; anything else raises an uncatchable
/// Objective-C exception (confirmed on real macos-14 hardware: `NSWindow
/// should only be instantiated on the main thread!`, via `-[NSWindow
/// _initContent:styleMask:backing:defer:contentView:]`). `Inner::create`
/// checks this before touching any Cocoa API so that calling it from the
/// wrong thread is a clean, catchable `Err` instead of a process abort —
/// this isn't just a test-environment quirk (`cargo test` runs every test
/// body on its own spawned thread, never the real main thread, which is
/// how this was first found), a real Fable program calling `window.create`
/// from inside a `worker` isolate would hit the identical crash.
fn is_main_thread() -> bool {
    unsafe { send0_bool(class("NSThread"), sel("isMainThread")) != OBJC_NO }
}

fn shared_app() -> *mut Object {
    unsafe { send0(class("NSApplication"), sel("sharedApplication")) }
}

fn ns_string(s: &str) -> *mut Object {
    // Safety: `stringWithUTF8String:` copies the bytes; the `CString` only
    // needs to outlive this call. Returns an autoreleased `NSString` (see
    // the module doc comment) — fine for the short, immediate uses this
    // file makes of the ones it creates (window title, run-loop mode
    // string), none of which are retained across a frame boundary.
    unsafe {
        let cs = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
        let cls = class("NSString");
        send1_ptr_ret(cls, sel("stringWithUTF8String:"), cs.as_ptr())
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
/// — never a guessed fixed buffer size. Mirrors `x11.rs`'s helper of the
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
/// `x11.rs`'s helper of the same name exactly (`GlFns` is `Copy`, so this
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

/// The real guts of a `WindowHandle` (see `src/window/mod.rs`) — an
/// `NSWindow` plus a current `NSOpenGLContext`.
pub struct Inner {
    window: *mut Object,
    ctx: *mut Object,
    gl: GlFns,
    /// `charactersIgnoringModifiers` text of keys currently held (inserted
    /// on `KeyDown`, removed on `KeyUp` — see [`poll`]), so `key_down(name)`
    /// can match by name the way `x11.rs` matches by `XStringToKeysym`. See
    /// `key_down`'s doc comment for the caveat this approach has that X11's
    /// keysym model doesn't.
    pressed: std::collections::HashSet<String>,
    pub mouse: (f64, f64),
    pub width: i32,
    pub height: i32,
    pub should_close: bool,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        if !is_main_thread() {
            return Err(
                "window.create: must run on the process's main thread (macOS requires all \
                 NSWindow/AppKit calls there); this fires from a `worker` isolate, or from \
                 any thread other than the one that started the program"
                    .to_string(),
            );
        }
        let gl = GlFns::load()?;
        ensure_app_init();

        // Safety: every call below follows the standard minimal Cocoa
        // "create a window with a GL-capable pixel format + context" recipe
        // (the direct analog of `x11.rs::create`'s GLX recipe); every
        // fallible step (a null return from `alloc`/`init...`) is checked
        // and anything already created is released before returning `Err`.
        unsafe {
            let style_mask = NS_WINDOW_STYLE_MASK_TITLED
                | NS_WINDOW_STYLE_MASK_CLOSABLE
                | NS_WINDOW_STYLE_MASK_MINIATURIZABLE
                | NS_WINDOW_STYLE_MASK_RESIZABLE;
            let rect = NsRect {
                origin: NsPoint { x: 0.0, y: 0.0 },
                size: NsSize {
                    width: w as f64,
                    height: h as f64,
                },
            };

            let window_class = class("NSWindow");
            let window_alloc = send0(window_class, sel("alloc"));
            if window_alloc.is_null() {
                return Err("window.create: [NSWindow alloc] returned nil".to_string());
            }
            let window = send_init_window(
                window_alloc,
                sel("initWithContentRect:styleMask:backing:defer:"),
                rect,
                style_mask,
                NS_BACKING_STORE_BUFFERED,
                OBJC_NO,
            );
            if window.is_null() {
                return Err(
                    "window.create: NSWindow initWithContentRect:... returned nil".to_string(),
                );
            }

            send1_obj(window, sel("setTitle:"), ns_string(title));

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
                send0_void(window, sel("release"));
                return Err("window.create: [NSOpenGLPixelFormat alloc] returned nil".to_string());
            }
            let fmt = send_init_pixel_format(fmt_alloc, sel("initWithAttributes:"), attrs.as_ptr());
            if fmt.is_null() {
                send0_void(window, sel("release"));
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
                send0_void(window, sel("release"));
                return Err("window.create: [NSOpenGLContext alloc] returned nil".to_string());
            }
            let ctx = send_init_context(
                ctx_alloc,
                sel("initWithFormat:shareContext:"),
                fmt,
                std::ptr::null_mut(),
            );
            // `fmt` is only needed for this call — release right after,
            // mirroring `x11.rs` freeing `XVisualInfo` right after
            // `glXCreateContext`.
            send0_void(fmt, sel("release"));
            if ctx.is_null() {
                send0_void(window, sel("release"));
                return Err(
                    "window.create: NSOpenGLContext initWithFormat:shareContext: returned nil"
                        .to_string(),
                );
            }

            let content_view = send0(window, sel("contentView"));
            send1_obj(ctx, sel("setView:"), content_view);
            send0_void(ctx, sel("makeCurrentContext"));

            send1_obj(window, sel("makeKeyAndOrderFront:"), std::ptr::null_mut());

            Ok(Inner {
                window,
                ctx,
                gl,
                pressed: std::collections::HashSet::new(),
                mouse: (0.0, 0.0),
                width: w,
                height: h,
                should_close: false,
            })
        }
    }

    pub fn poll(&mut self) {
        // Safety: `nextEventMatchingMask:untilDate:inMode:dequeue:` with
        // `untilDate: [NSDate distantPast]` is the standard non-blocking
        // idiom (return immediately with `nil` if nothing is queued) — the
        // direct structural analog of `x11.rs::poll`'s
        // `while XPending(display) > 0 { XNextEvent(...) }` loop: drain
        // everything currently queued, once per frame, never block.
        unsafe {
            let app = shared_app();
            let distant_past = send0(class("NSDate"), sel("distantPast"));
            let mode = ns_string("kCFRunLoopDefaultMode");
            loop {
                let event = send_next_event(
                    app,
                    sel("nextEventMatchingMask:untilDate:inMode:dequeue:"),
                    NS_ANY_EVENT_MASK,
                    distant_past,
                    mode,
                    OBJC_YES,
                );
                if event.is_null() {
                    break;
                }

                let event_type = send0_uint(event, sel("type"));
                match event_type {
                    NS_EVENT_TYPE_KEY_DOWN | NS_EVENT_TYPE_KEY_UP => {
                        // `charactersIgnoringModifiers` returns an `NSString*`
                        // (an object), NOT a C string — a second message,
                        // `UTF8String`, is needed to get an actual `char*`
                        // out of it. Reading the NSString object pointer
                        // itself as if it were a C string is not just wrong
                        // text: short strings use tagged pointers (the
                        // "pointer" packs the characters into the pointer
                        // bits, no heap address at all), so dereferencing it
                        // directly segfaults on essentially every keypress.
                        let ns_str = send0(event, sel("charactersIgnoringModifiers"));
                        if !ns_str.is_null() {
                            let chars_ptr = send0_ptr(ns_str, sel("UTF8String"));
                            if !chars_ptr.is_null() {
                                // Safety: `UTF8String`'s pointer is valid at
                                // least until the next autorelease pool
                                // drain, which (per the module doc comment)
                                // never happens mid-process here — still,
                                // copy into an owned `String` immediately
                                // rather than holding the raw pointer across
                                // any further calls, per the research
                                // brief's caution.
                                let s = std::ffi::CStr::from_ptr(chars_ptr)
                                    .to_string_lossy()
                                    .into_owned();
                                if event_type == NS_EVENT_TYPE_KEY_DOWN {
                                    self.pressed.insert(s);
                                } else {
                                    self.pressed.remove(&s);
                                }
                            }
                        }
                    }
                    NS_EVENT_TYPE_MOUSE_MOVED => {
                        // `locationInWindow` returns an `NSPoint` (two
                        // `f64`s) in the window's flipped-from-AppKit
                        // (bottom-left-origin) coordinate system; used
                        // as-is, matching `x11.rs`'s equally raw
                        // top-left-origin `XMotionEvent.x/y` — this
                        // namespace doesn't normalize coordinate origin
                        // conventions across platforms (a pre-existing
                        // asymmetry, not introduced here).
                        let f: unsafe extern "C" fn(*mut Object, SEL) -> NsPoint =
                            std::mem::transmute(objc_msgSend as *const ());
                        let p = f(event, sel("locationInWindow"));
                        self.mouse = (p.x, p.y);
                    }
                    _ => {}
                }

                send1_obj(app, sel("sendEvent:"), event);
            }

            // Close-button detection: with `Closable` set and no delegate
            // installed, AppKit's default `performClose:`/`close` path
            // (invoked when the user clicks the close box) orders the
            // window out unconditionally. Polling `isVisible` after the
            // pump is the simplest reliable signal without building a
            // runtime Objective-C subclass for a `NSWindowDelegate` — see
            // the module's research notes / task brief for why a delegate
            // was deliberately not built.
            if !self.should_close {
                let visible = send0_bool(self.window, sel("isVisible"));
                if visible == OBJC_NO {
                    self.should_close = true;
                }
            }

            // `contentView`'s frame reflects live resizes; keep width/height
            // in sync the same way `x11.rs::poll` updates them from
            // `CONFIGURE_NOTIFY`.
            let content_view = send0(self.window, sel("contentView"));
            if !content_view.is_null() {
                let f: unsafe extern "C" fn(*mut Object, SEL) -> NsRect =
                    std::mem::transmute(objc_msgSend as *const ());
                let frame = f(content_view, sel("frame"));
                self.width = frame.size.width as i32;
                self.height = frame.size.height as i32;
            }
        }
    }

    /// Matches by `charactersIgnoringModifiers` text (e.g. `"a"`, `" "`,
    /// arrow-key private-use characters AppKit assigns), inserted on
    /// `KeyDown`/removed on `KeyUp` in [`poll`]. **Semantic caveat** (called
    /// out explicitly in the research brief): this is text, not a
    /// hardware-key identity — unlike `x11.rs`'s `XStringToKeysym`/
    /// `XLookupKeysym` pairing, which resolves a *keysym name* independent
    /// of any live keyboard layout, `characters`-based matching is
    /// layout/shift-sensitive. Good enough for the same simple game/demo
    /// use this namespace targets on Linux; a `keyCode`-based (raw
    /// scancode) alternate API would be needed for strict physical-key
    /// tracking, and is left as a follow-up, not built here.
    pub fn key_down(&self, name: &str) -> bool {
        self.pressed.contains(name)
    }

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // Safety: makes this window's context current before issuing GL
        // calls — necessary if another `Window` made itself current since
        // this one was created (matches `x11.rs::clear`'s same caveat for
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
    // (`src/window/mod.rs`). Mirrors `x11.rs`'s equivalent block exactly,
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
    /// matches `x11.rs::teardown`'s ordering discipline. Does not release
    /// the process-lifetime `NSApplication`/autorelease pool/OpenGL
    /// framework handle, exactly as `x11.rs` never closes its X `Display`'s
    /// underlying `libGL.so.1` `dlopen` handle — those are process-lifetime
    /// resources, not per-window ones.
    pub fn teardown(self) {
        // Safety: `window`/`ctx` were each produced by a matching `alloc`+
        // `init...` pair in `Inner::create` and are still +1 owned (nothing
        // else in this file retains or releases them); releasing each
        // exactly once here, in the reverse order they were created, is
        // therefore balanced. `self` is consumed so this can't run twice.
        unsafe {
            send0_void(self.ctx, sel("release"));
            send0_void(self.window, sel("release"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end smoke test: create a window, clear it, swap buffers, pump
    /// events, confirm it isn't asking to close, then tear it down. Skips
    /// gracefully (doesn't panic the suite) if window/context creation
    /// fails for any environment-specific reason (e.g. a CI runner
    /// restricting window-server access) — mirrors `x11.rs`'s
    /// `create_clear_swap_poll_close` test's graceful-skip style, since
    /// `cargo test --features gl` must stay green even in constrained
    /// environments. In practice this always skips under `cargo test`
    /// itself: `Inner::create`'s main-thread check (see [`is_main_thread`])
    /// correctly rejects it, since `libtest` runs every test body on its
    /// own spawned thread, never the process's real main thread — a real
    /// Fable program (whose interpreter runs on the actual main thread
    /// unless the script explicitly uses `worker`) hits neither this check
    /// nor the AppKit crash it exists to prevent.
    #[test]
    fn create_clear_swap_poll_close() {
        let inner = match Inner::create("fable window test", 320, 240) {
            Ok(inner) => inner,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };
        let mut inner = inner;
        assert_eq!(inner.width, 320);
        assert_eq!(inner.height, 240);
        inner.clear(0.1, 0.2, 0.3, 1.0);
        inner.swap_buffers();
        inner.poll();
        assert!(!inner.should_close);
        inner.teardown();
    }
}
