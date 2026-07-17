//! X11/Xlib primitives shared by every Linux window backend — window
//! creation, the event-pump loop, and the async-protocol-error watch. None
//! of this references a rendering API at all (no GL/GLX, no Vulkan): each
//! backend module (`gl.rs`, `vulkan.rs`) composes an [`X11WindowState`] and
//! adds only its own rendering-specific pieces (a GLX context, or a Vulkan
//! surface/swapchain) around it. Split out of the original single-file
//! `x11.rs` when the Vulkan backend arrived — the same split
//! `macos/shared.rs` got when Metal arrived — so this machinery is written
//! and reasoned about exactly once.
//!
//! Struct layouts and function prototypes below were read directly from
//! `/usr/include/X11/{Xlib,X,Xutil}.h` in this container; see `gl.rs`'s
//! module docs for the GLX side's own corroboration story.
//!
//! **Linking strategy** (deliberate): X11 is linked normally
//! (`#[link(name = "X11")]`) against the system `libX11.so` —
//! `libx11-dev`-equivalent headers/libs are standard on any Linux desktop
//! dev machine, unlike GL/Vulkan dev packages, which vary a lot. Each
//! rendering API is resolved dynamically at runtime by its own backend
//! module instead (`dlopen("libGL.so.1")` in `gl.rs`;
//! `dlopen("libvulkan.so.1")` via `crate::vk` for Vulkan).

// Phase 0 of the Vulkan graphics arc: `vulkan.rs` is a stub that doesn't
// consume this module yet, so a `--features vulkan`-only build (`gl` off)
// sees these items as unused. `gl.rs` exercises all of them whenever `gl`
// is on, so scoping the allowance to "only when `gl` is off" keeps real
// dead-code detection active for every build that actually uses them.
// Remove when the Vulkan backend starts composing `X11WindowState`
// (Phase 1).
#![cfg_attr(not(feature = "gl"), allow(dead_code))]

use std::collections::HashSet;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void, CString};
use std::sync::atomic::{AtomicBool, Ordering};

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
pub(super) type XID = c_ulong;
pub(super) type Window = XID;
pub(super) type Colormap = XID;
pub(super) type Atom = XID;
pub(super) type KeySym = XID;
pub(super) type Time = c_ulong;
pub(super) type XBool = c_int; // Xlib's `Bool` is a plain `int`

/// `Xutil.h:287-302`. Field order/types confirmed on disk.
#[repr(C)]
pub(super) struct XVisualInfo {
    pub(super) visual: *mut Visual,
    pub(super) visualid: c_ulong,
    pub(super) screen: c_int,
    pub(super) depth: c_int,
    pub(super) class: c_int,
    pub(super) red_mask: c_ulong,
    pub(super) green_mask: c_ulong,
    pub(super) blue_mask: c_ulong,
    pub(super) colormap_size: c_int,
    pub(super) bits_per_rgb: c_int,
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
pub(super) struct XErrorEvent {
    type_: c_int,
    display: *mut Display,
    resourceid: XID,
    serial: c_ulong,
    error_code: u8,
    request_code: u8,
    minor_code: u8,
}

pub(super) type XErrorHandler = unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int;

/// Set by [`record_x_error`] while a temporary error handler is installed
/// during window creation (see `gl::Inner::create`). Xlib delivers protocol
/// errors (e.g. `BadMatch`/`BadWindow` from a misconfigured visual)
/// asynchronously — the request that caused one can return a normal-looking
/// value, with the error only surfacing later, typically on the next
/// server round-trip. Xlib's *default* handler calls `exit()`
/// unconditionally on any such error, which would take down the whole
/// Fable process — contrary to every other failure mode in this module (and
/// Fable's own convention that nothing panics the interpreter). Installing
/// this handler for the risky span of `create` and `XSync`-ing before
/// declaring success converts that into a normal, catchable `Err` instead.
pub(super) static X_PROTOCOL_ERROR: AtomicBool = AtomicBool::new(false);

pub(super) unsafe extern "C" fn record_x_error(
    _display: *mut Display,
    _event: *mut XErrorEvent,
) -> c_int {
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
pub(super) const X_FALSE: XBool = 0;
pub(super) const X_TRUE: XBool = 1;

#[link(name = "X11")]
extern "C" {
    pub(super) fn XOpenDisplay(display_name: *const c_char) -> *mut Display;
    pub(super) fn XCloseDisplay(display: *mut Display) -> c_int;
    pub(super) fn XDefaultScreen(display: *mut Display) -> c_int;
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
    pub(super) fn XFree(data: *mut c_void) -> c_int;
    pub(super) fn XSetErrorHandler(handler: Option<XErrorHandler>) -> Option<XErrorHandler>;
    pub(super) fn XSync(display: *mut Display, discard: XBool) -> c_int;
}

/// The X11 window plus polling-state (pressed keys, mouse position,
/// dimensions, close-requested flag) every backend composes by holding one
/// of these — exactly the state `win32.rs`'s `Inner` struct holds inline,
/// and the direct analog of `macos/shared.rs`'s `CocoaWindowState`.
/// Factored out only because Linux now has two backends that would
/// otherwise duplicate this window-creation/event-pump logic identically.
pub(super) struct X11WindowState {
    pub(super) display: *mut Display,
    pub(super) window: Window,
    colormap: Colormap,
    wm_delete: Atom,
    pressed: HashSet<KeySym>,
    pub(super) mouse: (f64, f64),
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) should_close: bool,
}

impl X11WindowState {
    /// Creates only the X window itself (colormap, title, `WM_DELETE_WINDOW`
    /// protocol, map) with the caller-chosen visual/depth — no GL/Vulkan
    /// context or surface. The GLX backend passes `glXChooseVisual`'s
    /// pick; a Vulkan backend can pass the screen's default visual.
    ///
    /// Infallible by design: `XCreateColormap`/`XCreateWindow` report
    /// misuse via *async* protocol errors, not return values, so the caller
    /// is responsible for the surrounding error watch —
    /// [`X_PROTOCOL_ERROR`]/[`record_x_error`] installed before this call,
    /// `XSync` + flag check after — and for tearing the returned state down
    /// (via [`X11WindowState::teardown`]) if its own setup fails partway.
    ///
    /// # Safety
    /// `display` must be a live Xlib connection and `visual`/`depth` a
    /// valid visual/depth pair for `screen` (e.g. straight out of a
    /// `XVisualInfo`), per `XCreateWindow`'s own contract.
    pub(super) unsafe fn create_window(
        display: *mut Display,
        screen: c_int,
        visual: *mut Visual,
        depth: c_int,
        title: &str,
        w: i32,
        h: i32,
    ) -> X11WindowState {
        let root = XRootWindow(display, screen);
        let colormap = XCreateColormap(display, root, visual, ALLOC_NONE);

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
            depth,
            INPUT_OUTPUT,
            visual,
            CW_COLORMAP | CW_BORDER_PIXEL | CW_EVENT_MASK,
            &mut attrs,
        );

        let title_c = CString::new(title).unwrap_or_else(|_| CString::new("").unwrap());
        XStoreName(display, window, title_c.as_ptr());

        let delete_name = CString::new("WM_DELETE_WINDOW").unwrap();
        let mut wm_delete = XInternAtom(display, delete_name.as_ptr(), X_FALSE);
        XSetWMProtocols(display, window, &mut wm_delete, 1);

        XMapWindow(display, window);

        X11WindowState {
            display,
            window,
            colormap,
            wm_delete,
            pressed: HashSet::new(),
            mouse: (0.0, 0.0),
            width: w,
            height: h,
            should_close: false,
        }
    }

    pub(super) fn poll(&mut self) {
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

    pub(super) fn key_down(&self, name: &str) -> bool {
        let Ok(cname) = CString::new(name) else {
            return false;
        };
        // Safety: `XStringToKeysym` takes a `const char*` and returns a
        // plain integer (`NoSymbol` = 0 for an unrecognized name); no
        // pointer is retained.
        let keysym = unsafe { XStringToKeysym(cname.as_ptr()) };
        keysym != 0 && self.pressed.contains(&keysym)
    }

    /// Destroys the X window, frees the colormap, and closes the display
    /// connection — the X11 half of a backend's teardown, called *after*
    /// the backend has destroyed its own rendering objects (a GLX context
    /// destroys against a live display). Idempotent-by-construction:
    /// `self` is consumed, so this can't run twice.
    pub(super) fn teardown(self) {
        // Safety: every handle here was produced by the matching X11 create
        // call in `create_window` and is torn down in the reverse order it
        // was created, exactly once (this method consumes `self`).
        unsafe {
            XDestroyWindow(self.display, self.window);
            XFreeColormap(self.display, self.colormap);
            XCloseDisplay(self.display);
        }
    }
}
