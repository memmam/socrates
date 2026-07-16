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
//!   (`glClearColor`/`glClear` here) need to be resolved from the framework.
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

// ---------------------------------------------------------------------------
// GL function pointers, resolved at runtime via dlopen/dlsym — mirrors
// `x11.rs`'s `GlFns` exactly, just against `OpenGL.framework` instead of
// `libGL.so.1`. Only the two draw calls this namespace needs; the GL
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

#[derive(Clone, Copy)]
struct GlFns {
    clear_color: FnClearColor,
    clear: FnClear,
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
            })
        }
    }
}

// ---------------------------------------------------------------------------
// NSApplication one-time setup — must happen once per process before any
// window is created. Also where the process-lifetime autorelease pool
// (see the module doc comment) is created.
// ---------------------------------------------------------------------------

/// Prints a step marker to stderr and flushes immediately. **Temporary**
/// bisection instrumentation for the macos.rs Cocoa-abort investigation
/// (see the CI failure history in PR #45): the AppKit crash this file has
/// hit so far is an unrecoverable process abort (`SIGABRT` via a foreign
/// exception Rust can't catch), which gives no Rust-level unwind/backtrace
/// to inspect — only whatever reached stderr before the abort. `eprintln!`
/// alone isn't provably enough (a `SIGABRT` can still race a buffered
/// write), hence the explicit flush after every marker. Remove once the
/// crashing call is identified and permanently fixed.
fn debug_step(label: &str) {
    eprintln!("[macos debug] {label}");
    let _ = std::io::Write::flush(&mut std::io::stderr());
}

fn ensure_app_init() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| unsafe {
        debug_step("ensure_app_init: start");
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
        debug_step("ensure_app_init: autorelease pool ready");

        let app_class = class("NSApplication");
        let app = send0(app_class, sel("sharedApplication"));
        debug_step("ensure_app_init: sharedApplication returned");
        // Return value (did the policy change take effect) intentionally
        // ignored — this mirrors x11.rs's "only check calls whose failure
        // blocks progress" convention; a regular-policy app that somehow
        // keeps its prior policy still creates and shows windows fine.
        let _ = send1_int_bool(
            app,
            sel("setActivationPolicy:"),
            NS_APPLICATION_ACTIVATION_POLICY_REGULAR,
        );
        debug_step("ensure_app_init: setActivationPolicy: returned");

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
        debug_step("ensure_app_init: activateIgnoringOtherApps: returned");
    });
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
            debug_step("create: NSWindow initWithContentRect:... returned");

            send1_obj(window, sel("setTitle:"), ns_string(title));
            debug_step("create: setTitle: returned");

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
            debug_step("create: NSOpenGLPixelFormat initWithAttributes: returned");

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
            debug_step("create: NSOpenGLContext initWithFormat:shareContext: returned");

            let content_view = send0(window, sel("contentView"));
            debug_step("create: contentView returned");
            send1_obj(ctx, sel("setView:"), content_view);
            debug_step("create: setView: returned");
            send0_void(ctx, sel("makeCurrentContext"));
            debug_step("create: makeCurrentContext returned");

            send1_obj(window, sel("makeKeyAndOrderFront:"), std::ptr::null_mut());
            debug_step("create: makeKeyAndOrderFront: returned");

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
    /// environments.
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
