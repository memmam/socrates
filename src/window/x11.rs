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

/// The handful of GLX/GL 1.0 entry points this namespace needs — the
/// GLEW-equivalent loader the later `gl` draw-call PR will extend with the
/// rest of the GL function table. Plain function pointers, `Copy`: the
/// underlying library load is process-wide and permanent (see [`load`]),
/// so there is nothing here for a `Drop` to release.
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
            Ok(GlFns {
                choose_visual: sym!("glXChooseVisual", FnChooseVisual),
                create_context: sym!("glXCreateContext", FnCreateContext),
                destroy_context: sym!("glXDestroyContext", FnDestroyContext),
                make_current: sym!("glXMakeCurrent", FnMakeCurrent),
                swap_buffers: sym!("glXSwapBuffers", FnSwapBuffers),
                get_current_context: sym!("glXGetCurrentContext", FnGetCurrentContext),
                clear_color: sym!("glClearColor", FnClearColor),
                clear: sym!("glClear", FnClear),
            })
        }
    }
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
