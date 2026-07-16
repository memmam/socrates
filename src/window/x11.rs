//! Linux/X11/GLX backend for the `window` namespace.
//!
//! **Linking strategy** (deliberate):
//! - X11 is linked normally (`#[link(name = "X11")]`) against the system
//!   `libX11.so` — `libx11-dev`-equivalent headers/libs are standard on any
//!   Linux desktop dev machine, unlike GL dev packages, which vary a lot.
//! - GL/GLX is resolved dynamically via `dlopen("libGL.so.1")` + `dlsym` at
//!   runtime. Many real target machines (this container included) have no
//!   `libGL.so` dev symlink/headers, so linking against GL statically would
//!   be fragile. This also matches the shape the later general `gl`
//!   draw-call namespace's function-pointer loader will extend.
//!
//! Struct layouts and function prototypes below were read directly from
//! `/usr/include/X11/{Xlib,X,Xutil}.h` in this container; GLX prototypes and
//! token values (stable, unrevised GLX 1.x ABI) are cross-corroborated from
//! multiple independent mirrors (GLEW, libglvnd, Mesa, Khronos refpages,
//! X.org man pages) rather than a single raw fetch of `glx.h` (this
//! session's egress policy blocked a direct fetch of it).
//!
//! `GlFns` also carries a GL 3.3 core-profile function table (shaders,
//! programs, buffers, VAOs, textures, uniforms, draw calls) beyond the
//! GLX/GL-1.0 handful above, for the upcoming backend-neutral `gfx`
//! draw-call namespace. Signatures and token values are cross-corroborated
//! against Khronos's own `glcorearb.h` and the Linux OpenGL ABI spec (which
//! guarantees `libGL.so.1` statically exports every entry point through GL
//! 1.2 — everything newer is resolved dynamically via `glXGetProcAddress`,
//! *including* GL-1.3-vintage `glActiveTexture`, which is one release past
//! that static-export floor).

use std::collections::HashSet;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CString};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// X11 types (Xlib.h / X.h / Xutil.h) — field layouts confirmed against the
// headers on disk in this container.
// ---------------------------------------------------------------------------

#[repr(C)]
pub struct Display {
    _private: [u8; 0],
}
#[repr(C)]
pub struct Visual {
    _private: [u8; 0],
}

#[allow(clippy::upper_case_acronyms)] // matches X11's own `XID` name
type XID = c_ulong;
type Window = XID;
type Colormap = XID;
type Atom = XID;
type KeySym = XID;
type Time = c_ulong;
type XBool = c_int; // Xlib's `Bool` is a plain `int`

/// `Xutil.h:287-302`. Field order/types confirmed on disk.
#[repr(C)]
struct XVisualInfo {
    visual: *mut Visual,
    visualid: c_ulong,
    screen: c_int,
    depth: c_int,
    class: c_int,
    red_mask: c_ulong,
    green_mask: c_ulong,
    blue_mask: c_ulong,
    colormap_size: c_int,
    bits_per_rgb: c_int,
}

/// `Xlib.h:290-306`.
#[repr(C)]
struct XSetWindowAttributes {
    background_pixmap: c_ulong,
    background_pixel: c_ulong,
    border_pixmap: c_ulong,
    border_pixel: c_ulong,
    bit_gravity: c_int,
    win_gravity: c_int,
    backing_store: c_int,
    backing_planes: c_ulong,
    backing_pixel: c_ulong,
    save_under: XBool,
    event_mask: c_long,
    do_not_propagate_mask: c_long,
    override_redirect: XBool,
    colormap: Colormap,
    cursor: XID,
}

/// `Xlib.h:558-571`.
#[repr(C)]
#[derive(Clone, Copy)]
struct XKeyEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: Time,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    keycode: c_uint,
    same_screen: XBool,
}

/// `Xlib.h:598-611` — same prefix as `XKeyEvent` through `state`.
#[repr(C)]
#[derive(Clone, Copy)]
struct XMotionEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    window: Window,
    root: Window,
    subwindow: Window,
    time: Time,
    x: c_int,
    y: c_int,
    x_root: c_int,
    y_root: c_int,
    state: c_uint,
    is_hint: c_char,
    same_screen: XBool,
}

/// `Xlib.h:768-780`.
#[repr(C)]
#[derive(Clone, Copy)]
struct XConfigureEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    event: Window,
    window: Window,
    x: c_int,
    y: c_int,
    width: c_int,
    height: c_int,
    border_width: c_int,
    above: Window,
    override_redirect: XBool,
}

/// `Xlib.h:897-910`. `data` is itself a C union (`b`/`s`/`l`); we represent
/// it as its largest variant (`l: [long; 5]`, 40 bytes) since only
/// `data.l[0]` is ever read (the `WM_DELETE_WINDOW` atom comparison) — that
/// matches the union's memory layout exactly.
#[repr(C)]
#[derive(Clone, Copy)]
struct XClientMessageEvent {
    type_: c_int,
    serial: c_ulong,
    send_event: XBool,
    display: *mut Display,
    window: Window,
    message_type: Atom,
    format: c_int,
    data_l: [c_long; 5],
}

/// `Xlib.h:973-1009`. `pad: [long; 24]` matches the real union's declared
/// padding exactly, so this union is binary-compatible with the real
/// `XEvent` even though we only name a handful of its members.
#[repr(C)]
#[derive(Clone, Copy)]
union XEvent {
    type_: c_int,
    key: XKeyEvent,
    motion: XMotionEvent,
    configure: XConfigureEvent,
    client: XClientMessageEvent,
    pad: [c_long; 24],
}

/// `Xlib.h:924-932`.
#[repr(C)]
struct XErrorEvent {
    type_: c_int,
    display: *mut Display,
    resourceid: XID,
    serial: c_ulong,
    error_code: u8,
    request_code: u8,
    minor_code: u8,
}

type XErrorHandler = unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int;

/// Set by [`record_x_error`] while a temporary error handler is installed
/// during window creation (see `Inner::create`). Xlib delivers protocol
/// errors (e.g. `BadMatch`/`BadWindow` from a misconfigured visual)
/// asynchronously — the request that caused one can return a normal-looking
/// value, with the error only surfacing later, typically on the next
/// server round-trip. Xlib's *default* handler calls `exit()`
/// unconditionally on any such error, which would take down the whole
/// Fable process — contrary to every other failure mode in this module (and
/// Fable's own convention that nothing panics the interpreter). Installing
/// this handler for the risky span of `create` and `XSync`-ing before
/// declaring success converts that into a normal, catchable `Err` instead.
static X_PROTOCOL_ERROR: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn record_x_error(_display: *mut Display, _event: *mut XErrorEvent) -> c_int {
    X_PROTOCOL_ERROR.store(true, Ordering::SeqCst);
    0
}

// X.h event-type / mask / misc constants (ground-truthed against
// /usr/include/X11/X.h in this container).
const KEY_PRESS: c_int = 2;
const KEY_RELEASE: c_int = 3;
const MOTION_NOTIFY: c_int = 6;
const CONFIGURE_NOTIFY: c_int = 22;
const CLIENT_MESSAGE: c_int = 33;

const KEY_PRESS_MASK: c_long = 1 << 0;
const KEY_RELEASE_MASK: c_long = 1 << 1;
const BUTTON_PRESS_MASK: c_long = 1 << 2;
const BUTTON_RELEASE_MASK: c_long = 1 << 3;
const POINTER_MOTION_MASK: c_long = 1 << 6;
const STRUCTURE_NOTIFY_MASK: c_long = 1 << 17;

const INPUT_OUTPUT: c_uint = 1;
const CW_BORDER_PIXEL: c_ulong = 1 << 3;
const CW_EVENT_MASK: c_ulong = 1 << 11;
const CW_COLORMAP: c_ulong = 1 << 13;
const ALLOC_NONE: c_int = 0;
const X_FALSE: XBool = 0;
const X_TRUE: XBool = 1;

// GLX_* visual attribute tokens (glx.h, stable since GLX 1.0).
const GLX_RGBA: c_int = 4;
const GLX_DOUBLEBUFFER: c_int = 5;
const GLX_DEPTH_SIZE: c_int = 12;

const GL_COLOR_BUFFER_BIT: c_uint = 0x0000_4000;

// GL 3.3-core enum tokens (gl.h / glcorearb.h; stable, unrevised values
// cross-corroborated against Khronos's own `glcorearb.h`) needed by the
// function table below and its future `gfx` namespace callers.
const GL_FALSE: u32 = 0x0000_0000;
// GL_TRUE/GL_NO_ERROR/GL_UNPACK_ALIGNMENT/GL_NEAREST/GL_REPEAT are part of
// the contracted GL 3.3-core token set but have no call site in the current
// `gfx` v1 surface (fixed to GL_FALSE, GL_LINEAR filtering, and
// GL_CLAMP_TO_EDGE wrapping only, and nothing yet exposes `glGetError` or
// `glPixelStorei`) — reserved for a fuller `gfx` API, not dead in the
// "never intended to be used" sense.
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

#[link(name = "X11")]
extern "C" {
    fn XOpenDisplay(display_name: *const c_char) -> *mut Display;
    fn XCloseDisplay(display: *mut Display) -> c_int;
    fn XDefaultScreen(display: *mut Display) -> c_int;
    fn XRootWindow(display: *mut Display, screen_number: c_int) -> Window;
    fn XCreateColormap(
        display: *mut Display,
        w: Window,
        visual: *mut Visual,
        alloc: c_int,
    ) -> Colormap;
    fn XFreeColormap(display: *mut Display, colormap: Colormap) -> c_int;
    #[allow(clippy::too_many_arguments)]
    fn XCreateWindow(
        display: *mut Display,
        parent: Window,
        x: c_int,
        y: c_int,
        width: c_uint,
        height: c_uint,
        border_width: c_uint,
        depth: c_int,
        class: c_uint,
        visual: *mut Visual,
        valuemask: c_ulong,
        attributes: *mut XSetWindowAttributes,
    ) -> Window;
    fn XDestroyWindow(display: *mut Display, w: Window) -> c_int;
    fn XMapWindow(display: *mut Display, w: Window) -> c_int;
    fn XStoreName(display: *mut Display, w: Window, window_name: *const c_char) -> c_int;
    fn XInternAtom(display: *mut Display, atom_name: *const c_char, only_if_exists: XBool)
        -> Atom;
    fn XSetWMProtocols(
        display: *mut Display,
        w: Window,
        protocols: *mut Atom,
        count: c_int,
    ) -> c_int;
    fn XNextEvent(display: *mut Display, event_return: *mut XEvent) -> c_int;
    fn XPending(display: *mut Display) -> c_int;
    fn XLookupKeysym(key_event: *mut XKeyEvent, index: c_int) -> KeySym;
    fn XStringToKeysym(string: *const c_char) -> KeySym;
    fn XFree(data: *mut c_void) -> c_int;
    fn XSetErrorHandler(handler: Option<XErrorHandler>) -> Option<XErrorHandler>;
    fn XSync(display: *mut Display, discard: XBool) -> c_int;
}

// dlopen/dlsym (libdl — merged into libc on modern glibc, but linked
// explicitly for portability to older glibc where they live in libdl.so).
// No `dlclose`: see `GlFns`'s doc comment — the library is loaded once and
// kept for the life of the process, never unloaded.
#[link(name = "dl")]
extern "C" {
    fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: c_int = 2;

// GLX/GL function pointer types, resolved at runtime via dlsym.
type GlxContext = *mut c_void;
type GlxDrawable = XID;
type FnChooseVisual = unsafe extern "C" fn(*mut Display, c_int, *mut c_int) -> *mut XVisualInfo;
type FnCreateContext =
    unsafe extern "C" fn(*mut Display, *mut XVisualInfo, GlxContext, XBool) -> GlxContext;
type FnDestroyContext = unsafe extern "C" fn(*mut Display, GlxContext);
type FnMakeCurrent = unsafe extern "C" fn(*mut Display, GlxDrawable, GlxContext) -> XBool;
type FnSwapBuffers = unsafe extern "C" fn(*mut Display, GlxDrawable);
type FnGetCurrentContext = unsafe extern "C" fn() -> GlxContext;
type FnClearColor = unsafe extern "C" fn(f32, f32, f32, f32);
type FnClear = unsafe extern "C" fn(c_uint);

// GL 3.3 core-profile function pointer types, resolved at runtime — 12 via
// `dlsym` (statically exported by `libGL.so.1` through the GL 1.2 ABI
// floor) and 32 via `glXGetProcAddress` (see `FnGetProcAddress` and the
// `proc_addr!` macro in `GlFns::load_uncached` below). GLenum/GLuint/GLint/
// GLsizei map to `u32`/`i32`, GLboolean to `u8`, GLfloat to `f32`, and
// GLsizeiptr to `isize` (pointer-width) — all fixed-width regardless of
// platform C ABI, unlike the `c_long`-based Xlib/GLX structs above.
type FnGetProcAddress = unsafe extern "C" fn(*const u8) -> *mut c_void;

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

/// The GLX/GL 1.0 entry points this namespace's window plumbing needs, plus
/// a GL 3.3 core-profile function table (shaders/programs/buffers/VAOs/
/// textures/uniforms/draw calls) for the backend-neutral `gfx` draw-call
/// namespace that consumes it (a separate, later PR — this struct and its
/// loader are the GLEW-equivalent groundwork). Plain function pointers,
/// `Copy`: the underlying library load is process-wide and permanent (see
/// [`load`]), so there is nothing here for a `Drop` to release.
#[derive(Clone, Copy)]
struct GlFns {
    choose_visual: FnChooseVisual,
    create_context: FnCreateContext,
    destroy_context: FnDestroyContext,
    make_current: FnMakeCurrent,
    swap_buffers: FnSwapBuffers,
    get_current_context: FnGetCurrentContext,
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
    // Resolved (and part of the contracted GL 3.3-core table) but not yet
    // called anywhere: no `gfx.*` member currently surfaces raw GL error
    // state to Fable. Reserved for a fuller `gfx` API.
    #[allow(dead_code)]
    get_error: FnGetError,
    read_pixels: FnReadPixels,
}

impl GlFns {
    /// `dlopen("libGL.so.1")` and resolve every symbol this module needs —
    /// but only ever once per process, cached in a `OnceLock`. Real GL
    /// drivers commonly initialize global state the first time their
    /// library is loaded that does not tolerate being `dlopen`/`dlclose`d
    /// repeatedly within one process (the documented reason GLFW/SDL never
    /// `dlclose` `libGL` once loaded, even though they create and destroy
    /// many GL contexts over a process's lifetime) — so the library itself
    /// is loaded once and kept forever, while GLX *contexts* on top of it
    /// are still created and destroyed freely, once per `Window`. Any
    /// failure (missing lib, missing symbol) is a clean `Err`, not a panic,
    /// and — like the rest of `window.create` — is cached too, so a broken
    /// driver reports the same error on every subsequent attempt rather
    /// than retrying a `dlopen` known to fail.
    fn load() -> Result<Self, String> {
        static CACHE: OnceLock<Result<GlFns, String>> = OnceLock::new();
        CACHE.get_or_init(Self::load_uncached).clone()
    }

    fn load_uncached() -> Result<GlFns, String> {
        // Safety: `dlopen`/`dlsym` are straightforward C FFI; every pointer
        // we hand them is a valid, NUL-terminated `CString`, and we check
        // every returned pointer before using it. `lib` is intentionally
        // never `dlclose`d (see the doc comment above) — it is a raw handle
        // used only to resolve symbols, not stored past this function.
        unsafe {
            let libname = CString::new("libGL.so.1").unwrap();
            let lib = dlopen(libname.as_ptr(), RTLD_NOW);
            if lib.is_null() {
                return Err(
                    "window.create: dlopen(\"libGL.so.1\") failed — no GL driver installed?"
                        .to_string(),
                );
            }
            macro_rules! sym {
                ($name:literal, $ty:ty) => {{
                    let cname = CString::new($name).unwrap();
                    let p = dlsym(lib, cname.as_ptr());
                    if p.is_null() {
                        return Err(format!(
                            "window.create: libGL.so.1 is missing the symbol `{}`",
                            $name
                        ));
                    }
                    std::mem::transmute::<*mut c_void, $ty>(p)
                }};
            }

            // `glXGetProcAddress` is itself a statically-exported symbol of
            // `libGL.so.1` (unlike the GL 3.x+ entry points it resolves), so
            // it's fetched with the same `dlsym` as everything above. This
            // file resolves the plain (non-`ARB`) name: both are exported
            // and behave identically on every driver this container's
            // 32-fn proc-address list below has been checked against
            // (Mesa, NVIDIA's proprietary driver via libglvnd), but
            // `glXGetProcAddress` is the one promoted into core GLX 1.4, so
            // it's the one used consistently everywhere in this file rather
            // than mixing it with the `ARB`-suffixed extension name.
            let get_proc_address: FnGetProcAddress = sym!("glXGetProcAddress", FnGetProcAddress);

            // Callable with no GLX context current (unlike `wglGetProcAddress`
            // on Windows, which requires one) — see `glXGetProcAddress`'s own
            // spec. `GLubyte*`, not `char*`, hence the cast to `*const u8`
            // rather than reusing `CString::as_ptr`'s `*const c_char` as-is.
            macro_rules! proc_addr {
                ($name:literal, $ty:ty) => {{
                    let cname = CString::new($name).unwrap();
                    let p = get_proc_address(cname.as_ptr() as *const u8);
                    if p.is_null() {
                        return Err(format!(
                            "window.create: glXGetProcAddress could not resolve `{}` — GL \
                             driver too old? (gfx needs GL 3.3 core profile)",
                            $name
                        ));
                    }
                    std::mem::transmute::<*mut c_void, $ty>(p)
                }};
            }

            Ok(GlFns {
                choose_visual: sym!("glXChooseVisual", FnChooseVisual),
                create_context: sym!("glXCreateContext", FnCreateContext),
                destroy_context: sym!("glXDestroyContext", FnDestroyContext),
                make_current: sym!("glXMakeCurrent", FnMakeCurrent),
                swap_buffers: sym!("glXSwapBuffers", FnSwapBuffers),
                get_current_context: sym!("glXGetCurrentContext", FnGetCurrentContext),
                clear_color: sym!("glClearColor", FnClearColor),
                clear: sym!("glClear", FnClear),

                // SHADERS (all proc-address: GLSL/shader objects are GL 2.0)
                create_shader: proc_addr!("glCreateShader", FnCreateShader),
                shader_source: proc_addr!("glShaderSource", FnShaderSource),
                compile_shader: proc_addr!("glCompileShader", FnCompileShader),
                get_shader_iv: proc_addr!("glGetShaderiv", FnGetShaderiv),
                get_shader_info_log: proc_addr!("glGetShaderInfoLog", FnGetShaderInfoLog),
                delete_shader: proc_addr!("glDeleteShader", FnDeleteShader),

                // PROGRAMS (all proc-address: GL 2.0)
                create_program: proc_addr!("glCreateProgram", FnCreateProgram),
                attach_shader: proc_addr!("glAttachShader", FnAttachShader),
                link_program: proc_addr!("glLinkProgram", FnLinkProgram),
                get_program_iv: proc_addr!("glGetProgramiv", FnGetProgramiv),
                get_program_info_log: proc_addr!("glGetProgramInfoLog", FnGetProgramInfoLog),
                use_program: proc_addr!("glUseProgram", FnUseProgram),
                delete_program: proc_addr!("glDeleteProgram", FnDeleteProgram),

                // BUFFERS (all proc-address: VBOs are GL 1.5)
                gen_buffers: proc_addr!("glGenBuffers", FnGenBuffers),
                bind_buffer: proc_addr!("glBindBuffer", FnBindBuffer),
                buffer_data: proc_addr!("glBufferData", FnBufferData),
                delete_buffers: proc_addr!("glDeleteBuffers", FnDeleteBuffers),

                // VAO (all proc-address: VAOs are GL 3.0)
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

                // TEXTURES: gen/bind/delete/tex_image_2d/tex_parameter_i are
                // GL 1.0/1.1 (direct dlsym); active_texture is GL 1.3, one
                // release past the static-export ABI floor (proc-address).
                gen_textures: sym!("glGenTextures", FnGenTextures),
                bind_texture: sym!("glBindTexture", FnBindTexture),
                tex_image_2d: sym!("glTexImage2D", FnTexImage2D),
                tex_parameter_i: sym!("glTexParameteri", FnTexParameteri),
                active_texture: proc_addr!("glActiveTexture", FnActiveTexture),
                delete_textures: sym!("glDeleteTextures", FnDeleteTextures),

                // UNIFORMS (all proc-address: GL 2.0)
                get_uniform_location: proc_addr!("glGetUniformLocation", FnGetUniformLocation),
                uniform_1i: proc_addr!("glUniform1i", FnUniform1i),
                uniform_1f: proc_addr!("glUniform1f", FnUniform1f),
                uniform_2f: proc_addr!("glUniform2f", FnUniform2f),
                uniform_3f: proc_addr!("glUniform3f", FnUniform3f),
                uniform_4f: proc_addr!("glUniform4f", FnUniform4f),
                uniform_matrix_4fv: proc_addr!("glUniformMatrix4fv", FnUniformMatrix4fv),

                // DRAW/MISC: all GL 1.0/1.1 (direct dlsym).
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

/// `GfxBufferKind` (see `src/window/mod.rs`) → the GL enum for that binding
/// target, kept private to this file like every other raw GL constant.
fn gl_buffer_target(kind: crate::window::GfxBufferKind) -> u32 {
    match kind {
        crate::window::GfxBufferKind::Vertex => GL_ARRAY_BUFFER,
        crate::window::GfxBufferKind::Index => GL_ELEMENT_ARRAY_BUFFER,
    }
}

/// Fetch a GL info log sized exactly via a prior `GL_INFO_LOG_LENGTH` query
/// (`log_len` includes the NUL terminator, or is 0/1 for an empty log) —
/// never a guessed fixed buffer size.
///
/// Safety: `get_log` must be a valid call into `glGetShaderInfoLog`/
/// `glGetProgramInfoLog` (or equivalent) writing at most `log_len` bytes
/// (including the NUL) into the buffer it's given.
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

/// Compile one shader stage (`GL_VERTEX_SHADER`/`GL_FRAGMENT_SHADER`);
/// `Err` carries the driver's compile log, sized via `GL_INFO_LOG_LENGTH`.
/// Takes `gl` by value (`GlFns` is `Copy`) so it needs no `&Inner` borrow.
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

/// The real guts of a `WindowHandle` (see `src/window/mod.rs`) — an X11
/// window plus a current-capable GLX context.
pub struct Inner {
    display: *mut Display,
    window: Window,
    colormap: Colormap,
    ctx: GlxContext,
    gl: GlFns,
    wm_delete: Atom,
    pressed: HashSet<KeySym>,
    pub mouse: (f64, f64),
    pub width: i32,
    pub height: i32,
    pub should_close: bool,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        let gl = GlFns::load()?;

        // Safety: every call below follows the standard minimal GLX
        // "create a window with a GL-capable visual" recipe (the same one
        // GLFW/SDL use on X11); every fallible step is checked and every
        // resource created before a later failure is torn down before
        // returning `Err`, so no partial window/context/colormap leaks.
        unsafe {
            let display = XOpenDisplay(ptr::null());
            if display.is_null() {
                return Err(
                    "window.create: XOpenDisplay failed (no X server / $DISPLAY not set?)"
                        .to_string(),
                );
            }

            let screen = XDefaultScreen(display);

            let mut attrib_list =
                [GLX_RGBA, GLX_DEPTH_SIZE, 24, GLX_DOUBLEBUFFER, 0 /* None */];
            let vi = (gl.choose_visual)(display, screen, attrib_list.as_mut_ptr());
            if vi.is_null() {
                XCloseDisplay(display);
                return Err(
                    "window.create: glXChooseVisual found no matching GL visual".to_string()
                );
            }

            // From here on, requests (XCreateWindow/XCreateColormap in
            // particular) can provoke an async X protocol error that Xlib
            // delivers later rather than as a return value. Install a
            // temporary handler (see `record_x_error`'s doc comment) so
            // that ends up a catchable `Err` instead of Xlib's default
            // handler's unconditional `exit()`. Every exit from here on
            // restores the previous handler before returning.
            X_PROTOCOL_ERROR.store(false, Ordering::SeqCst);
            let prev_handler = XSetErrorHandler(Some(record_x_error));

            let root = XRootWindow(display, screen);
            let colormap = XCreateColormap(display, root, (*vi).visual, ALLOC_NONE);

            let mut attrs: XSetWindowAttributes = std::mem::zeroed();
            attrs.colormap = colormap;
            attrs.border_pixel = 0;
            attrs.event_mask = KEY_PRESS_MASK
                | KEY_RELEASE_MASK
                | BUTTON_PRESS_MASK
                | BUTTON_RELEASE_MASK
                | POINTER_MOTION_MASK
                | STRUCTURE_NOTIFY_MASK;

            let window = XCreateWindow(
                display,
                root,
                0,
                0,
                w as c_uint,
                h as c_uint,
                0,
                (*vi).depth,
                INPUT_OUTPUT,
                (*vi).visual,
                CW_COLORMAP | CW_BORDER_PIXEL | CW_EVENT_MASK,
                &mut attrs,
            );

            let title_c = CString::new(title).unwrap_or_else(|_| CString::new("").unwrap());
            XStoreName(display, window, title_c.as_ptr());

            let delete_name = CString::new("WM_DELETE_WINDOW").unwrap();
            let mut wm_delete = XInternAtom(display, delete_name.as_ptr(), X_FALSE);
            XSetWMProtocols(display, window, &mut wm_delete, 1);

            XMapWindow(display, window);

            let ctx = (gl.create_context)(display, vi, ptr::null_mut(), X_TRUE);
            if ctx.is_null() {
                XFree(vi as *mut c_void);
                XDestroyWindow(display, window);
                XFreeColormap(display, colormap);
                XSetErrorHandler(prev_handler);
                XCloseDisplay(display);
                return Err("window.create: glXCreateContext failed".to_string());
            }

            if (gl.make_current)(display, window as GlxDrawable, ctx) == X_FALSE {
                (gl.destroy_context)(display, ctx);
                XFree(vi as *mut c_void);
                XDestroyWindow(display, window);
                XFreeColormap(display, colormap);
                XSetErrorHandler(prev_handler);
                XCloseDisplay(display);
                return Err("window.create: glXMakeCurrent failed".to_string());
            }

            XFree(vi as *mut c_void);

            // Force delivery of any protocol error the requests above
            // provoked (XCreateWindow/XCreateColormap are the risky ones —
            // e.g. a depth/visual mismatch reports BadMatch) before trusting
            // this window is actually usable.
            XSync(display, X_FALSE);
            let protocol_error = X_PROTOCOL_ERROR.load(Ordering::SeqCst);
            XSetErrorHandler(prev_handler);
            if protocol_error {
                (gl.destroy_context)(display, ctx);
                XDestroyWindow(display, window);
                XFreeColormap(display, colormap);
                XCloseDisplay(display);
                return Err(
                    "window.create: an X protocol error occurred while creating the window \
                     (misconfigured visual?)"
                        .to_string(),
                );
            }

            Ok(Inner {
                display,
                window,
                colormap,
                ctx,
                gl,
                wm_delete,
                pressed: HashSet::new(),
                mouse: (0.0, 0.0),
                width: w,
                height: h,
                should_close: false,
            })
        }
    }

    pub fn poll(&mut self) {
        // Safety: `XPending`/`XNextEvent` are the standard Xlib event-pump
        // pair; `ev` is zero-initialized before `XNextEvent` fills it in, so
        // reading any union field it didn't touch (we only ever read the
        // field matching `ev.type_`) is still defined (all-zero bytes).
        unsafe {
            while XPending(self.display) > 0 {
                let mut ev: XEvent = std::mem::zeroed();
                XNextEvent(self.display, &mut ev);
                match ev.type_ {
                    KEY_PRESS | KEY_RELEASE => {
                        let mut key_ev = ev.key;
                        let keysym = XLookupKeysym(&mut key_ev, 0);
                        if ev.type_ == KEY_PRESS {
                            self.pressed.insert(keysym);
                        } else {
                            self.pressed.remove(&keysym);
                        }
                    }
                    MOTION_NOTIFY => {
                        self.mouse = (ev.motion.x as f64, ev.motion.y as f64);
                    }
                    CONFIGURE_NOTIFY => {
                        self.width = ev.configure.width;
                        self.height = ev.configure.height;
                    }
                    CLIENT_MESSAGE if ev.client.data_l[0] as u64 == self.wm_delete => {
                        self.should_close = true;
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn key_down(&self, name: &str) -> bool {
        let Ok(cname) = CString::new(name) else {
            return false;
        };
        // Safety: `XStringToKeysym` takes a `const char*` and returns a
        // plain integer (`NoSymbol` = 0 for an unrecognized name); no
        // pointer is retained.
        let keysym = unsafe { XStringToKeysym(cname.as_ptr()) };
        keysym != 0 && self.pressed.contains(&keysym)
    }

    // Method-shaped accessors alongside the public fields above: macOS's
    // `Inner` is a two-backend enum (see `macos/mod.rs`), which can't expose
    // shared state via dot-field syntax across variants the way a plain
    // struct can — `window/mod.rs`'s generic `WindowHandle` code calls these
    // uniformly across all three platforms instead. Field and method share a
    // name safely (separate namespaces: `.should_close` is the field,
    // `.should_close()` is this method).
    pub fn mouse(&self) -> (f64, f64) {
        self.mouse
    }
    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
    pub fn should_close(&self) -> bool {
        self.should_close
    }
    /// This backend is always OpenGL/GLX — no Metal-equivalent exists on
    /// Linux, so unlike macOS's `Inner` enum this never varies.
    pub fn backend_name(&self) -> &'static str {
        "opengl"
    }

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // Safety: makes this window's context current before issuing GL
        // calls — necessary if another `Window` made itself current since
        // this one was created (GLX contexts are current per-thread, not
        // per-window). `glXMakeCurrent` can fail (e.g. a display-driver
        // reset) — skip the GL calls rather than issue them with no context
        // bound, which the GL spec leaves undefined.
        unsafe {
            if (self.gl.make_current)(self.display, self.window as GlxDrawable, self.ctx)
                != X_FALSE
            {
                (self.gl.clear_color)(r, g, b, a);
                (self.gl.clear)(GL_COLOR_BUFFER_BIT);
            }
        }
    }

    pub fn swap_buffers(&mut self) {
        // Safety: same current-context caveat as `clear`.
        unsafe {
            if (self.gl.make_current)(self.display, self.window as GlxDrawable, self.ctx)
                != X_FALSE
            {
                (self.gl.swap_buffers)(self.display, self.window as GlxDrawable);
            }
        }
    }

    // -----------------------------------------------------------------
    // gfx.* (v0.8) — GL 3.3 core-profile draw calls against this window's
    // context, consumed through `WindowHandle`'s `gl_*` wrappers
    // (`src/window/mod.rs`). Every method here re-asserts the context is
    // current first (`ensure_current`), exactly like `clear`/`swap_buffers`
    // above, and no-ops (returning a zero/default value where one is
    // needed) if that fails, rather than issuing GL calls with no context
    // bound.
    // -----------------------------------------------------------------

    /// Makes this window's context current on this thread; returns whether
    /// it succeeded. Shared by `clear`/`swap_buffers` above conceptually
    /// (they inline the same call) and every gfx method below.
    fn ensure_current(&mut self) -> bool {
        unsafe {
            (self.gl.make_current)(self.display, self.window as GlxDrawable, self.ctx) != X_FALSE
        }
    }

    /// Exposed as `win.make_current()` (v0.8): the same make-current call
    /// `clear()`/`swap_buffers()` already issue internally per call, just
    /// available on its own so `gfx.*` natives have an explicit window to
    /// target (see `Vm::gfx_current_window`).
    pub fn make_current(&mut self) {
        self.ensure_current();
    }

    /// Compiles + links a vertex/fragment GLSL pair. `Err` carries the
    /// driver's compile/link info log; any shader/program object created
    /// before the failure is cleaned up (deleted) before returning.
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
            // Flag both stages for deletion now that they're linked in:
            // GL keeps an attached, delete-flagged shader alive until it's
            // detached or the program itself is deleted, so this is safe
            // and is the standard cleanup idiom (no need to keep the
            // shader objects around once the program links successfully).
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

    /// Fixed to `GL_FLOAT`, `normalized = GL_FALSE` (v1 scope: `f32` vertex
    /// data only); `stride`/`offset` are byte counts, `offset` cast to a
    /// pointer per `glVertexAttribPointer`'s ABI (a real address only when
    /// no buffer is bound to `GL_ARRAY_BUFFER` — always bound here).
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

    /// `GL_RGBA`/`GL_RGB` by `has_alpha`, always `GL_UNSIGNED_BYTE`; also
    /// sets `GL_LINEAR` filtering and `GL_CLAMP_TO_EDGE` wrapping — fixed
    /// defaults, matching the `gfx` v1 scope.
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

    /// Binds `program` first (uniform locations are only meaningful
    /// relative to whichever program is currently in use), then resolves
    /// `name`. Shared by every `set_uniform_*` method below.
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

    /// `values` is 16 column-major floats (`std.glm`'s `Mat4` is already
    /// column-major, matching GL's own convention) — `transpose = GL_FALSE`.
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

    /// `byte_offset` is a byte offset into the bound `GL_ELEMENT_ARRAY_BUFFER`
    /// cast to a pointer (real GL ABI — never an actual address), `u32`
    /// indices (`GL_UNSIGNED_INT`) fixed, per the `gfx` v1 scope.
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

    /// `glClearColor` + `glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)`
    /// — unlike `clear()` above (color only), `gfx.clear` also clears depth.
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

    /// `GL_RGBA`/`GL_UNSIGNED_BYTE` into a freshly allocated `w * h * 4`
    /// byte buffer — for pixel-spot-check golden tests (this repo's
    /// verification convention for rendered output).
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
    /// destroy the GL context, destroy the X window, free the colormap,
    /// close the display connection.
    pub fn teardown(self) {
        // Safety: every handle here was produced by the matching X11/GLX
        // create call in `Inner::create` and is torn down in the reverse
        // order it was created, exactly once (this method consumes `self`).
        unsafe {
            if (self.gl.get_current_context)() == self.ctx {
                (self.gl.make_current)(self.display, 0, ptr::null_mut());
            }
            (self.gl.destroy_context)(self.display, self.ctx);
            XDestroyWindow(self.display, self.window);
            XFreeColormap(self.display, self.colormap);
            XCloseDisplay(self.display);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end smoke test: create a window, clear it, swap buffers, pump
    /// events, confirm it isn't asking to close, then tear it down. Skips
    /// gracefully (doesn't panic the suite) when there's no display to open
    /// — `cargo test --features gl` must stay green in headless CI/dev
    /// environments too.
    #[test]
    fn create_clear_swap_poll_close() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("skipping: $DISPLAY not set");
            return;
        }
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
