//! The Objective-C runtime dispatch layer shared by every macOS raw-FFI
//! backend — originally private to `window/macos/shared.rs`, promoted to a
//! crate-level module when its second consumer arrived (the `gpu`
//! namespace's native Metal compute path), per CLAUDE.md's shared-core
//! rule: abstractions are extracted when real duplication appears, not
//! guessed up front. Consumers: `window/macos/{shared,gl,metal}.rs`
//! (Cocoa windowing + both rendering backends) and `gpu.rs`'s Metal
//! compute section (via `mtl.rs`).
//!
//! **Why arm64-only**: x86_64 Macs require picking between `objc_msgSend`
//! and `objc_msgSend_stret` per call site depending on whether the returned
//! struct fits in registers — a second, easy-to-get-wrong dispatch path.
//! Apple's arm64 ABI returns small aggregates (including `NsRect`, the one
//! struct-returning message the windowing layer sends) directly in
//! `x0`/`x1`/`v0`-`v3`, and `objc_msgSend_stret` doesn't exist in the arm64
//! SDK at all — so plain `objc_msgSend`, correctly `transmute`d per call
//! shape, is the *only* path and there is no split to get wrong. This
//! matches the release matrix (`aarch64-apple-darwin` is the only macOS
//! target `socrates build` staples for), so nothing is lost by declining
//! x86_64.
//!
//! **`objc_msgSend` dispatch**: Rust has no variadic FFI, so there is no
//! single Rust signature for `objc_msgSend` — the raw symbol is declared
//! once below and `transmute`d to a distinct `unsafe extern "C" fn(...)`
//! type at every call site, one per distinct argument/return shape. Rather
//! than one generic macro, every shape used anywhere in the crate gets its
//! own small named wrapper below — clearer to audit than a cleverer
//! variadic-emulating macro, which matters more here than brevity given the
//! risk profile (selector/argument-encoding mistakes compile cleanly and
//! only misbehave at runtime). Wrappers whose argument types are specific
//! to one API (AppKit window init, Metal clear colors/viewports/regions)
//! live next to their sole consumer instead; only genuinely shared shapes
//! live here.

// The Metal-only consumers (window/macos/metal.rs, and gpu.rs's native
// compute path) are the sole users of several shapes below (plus
// AutoreleasePool and nsstring_to_owned), so a gl-only build would see them
// as dead code. Scoping the allowance to "only when metal is off" keeps
// real dead-code detection active for every build that compiles all
// consumers — the same precision discipline window/macos/shared.rs used
// during the Metal backend's own phase-in.
#![cfg_attr(not(feature = "metal"), allow(dead_code))]

use std::ffi::{c_char, c_void, CString};

#[allow(clippy::upper_case_acronyms)] // matches Objective-C's own `SEL` name
pub(crate) type SEL = *mut c_void;
pub(crate) type Class = *mut Object;
#[repr(C)]
pub(crate) struct Object {
    _private: [u8; 0],
}

// `NSInteger`/`NSUInteger` are 64-bit on arm64; `BOOL` is a signed byte
// (`c_char`/`i8`) on Apple's modern ("NeXT") runtime, not `c_int` — a common
// mistake porting 32-bit-era Objective-C snippets.
pub(crate) type NsInteger = i64;
pub(crate) type NsUInteger = u64;
pub(crate) type ObjcBool = i8;
pub(crate) const OBJC_YES: ObjcBool = 1;
pub(crate) const OBJC_NO: ObjcBool = 0;

/// `CGFloat` is `f64` on every 64-bit Apple platform (arm64 included).
pub(crate) type CgFloat = f64;

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NsPoint {
    pub(crate) x: CgFloat,
    pub(crate) y: CgFloat,
}
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NsSize {
    pub(crate) width: CgFloat,
    pub(crate) height: CgFloat,
}
/// On arm64 this 32-byte-aggregate return needs no `objc_msgSend_stret`
/// special-casing (see the module doc comment) — only the correctly typed
/// `transmute` at the call site.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct NsRect {
    pub(crate) origin: NsPoint,
    pub(crate) size: NsSize,
}

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> SEL;
    /// Never called directly — every call site `transmute`s this to the
    /// exact argument/return shape it needs (Rust has no variadic FFI; see
    /// the module doc comment).
    pub(crate) fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

/// RAII autorelease pool. The Cocoa event-pump layer gets away with one
/// process-lifetime pool (see `window/macos/shared.rs`'s memory-management
/// docs), but Metal frame/compute paths must drain autoreleased objects
/// promptly — `CAMetalLayer`'s drawables in particular are a small fixed
/// pool the layer only reclaims on actual release, so a leak there turns
/// into a deadlock, not just growth.
pub(crate) struct AutoreleasePool(*mut c_void);
impl AutoreleasePool {
    pub(crate) fn push() -> AutoreleasePool {
        AutoreleasePool(unsafe { objc_autoreleasePoolPush() })
    }
}
impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

/// Resolve a class by name, panicking only if the OS itself is missing the
/// class (would indicate a broken/ancient OS, not a recoverable condition —
/// classes looked up by name with no fallback path have no sensible
/// `Result` to return through, since every caller is inside `unsafe` setup
/// code already committed to the framework existing).
pub(crate) unsafe fn class(name: &str) -> Class {
    let cname = CString::new(name).unwrap();
    objc_getClass(cname.as_ptr())
}

pub(crate) unsafe fn sel(name: &str) -> SEL {
    let cname = CString::new(name).unwrap();
    sel_registerName(cname.as_ptr())
}

pub(crate) fn ns_string(s: &str) -> *mut Object {
    // Safety: `stringWithUTF8String:` copies the bytes; the `CString` only
    // needs to outlive this call. Returns an autoreleased `NSString` — every
    // caller either holds an [`AutoreleasePool`] or makes short, immediate
    // use under the windowing layer's process-lifetime pool.
    unsafe {
        let cs = CString::new(s).unwrap_or_else(|_| CString::new("").unwrap());
        let cls = class("NSString");
        send1_ptr_ret(cls, sel("stringWithUTF8String:"), cs.as_ptr())
    }
}

/// Copy an `NSString` into an owned Rust `String` (via `UTF8String` — the
/// NSString pointer itself must never be read as text: short strings are
/// tagged pointers with no heap address at all).
pub(crate) unsafe fn nsstring_to_owned(ns: *mut Object) -> String {
    if ns.is_null() {
        return String::new();
    }
    let p = send0_ptr(ns, sel("UTF8String"));
    if p.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// Shared `objc_msgSend` shapes.
// ---------------------------------------------------------------------------

pub(crate) unsafe fn send0(recv: *mut Object, s: SEL) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
pub(crate) unsafe fn send0_bool(recv: *mut Object, s: SEL) -> ObjcBool {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> ObjcBool =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
pub(crate) unsafe fn send0_uint(recv: *mut Object, s: SEL) -> NsUInteger {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> NsUInteger =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
pub(crate) unsafe fn send0_ptr(recv: *mut Object, s: SEL) -> *const c_char {
    let f: unsafe extern "C" fn(*mut Object, SEL) -> *const c_char =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
pub(crate) unsafe fn send0_void(recv: *mut Object, s: SEL) {
    let f: unsafe extern "C" fn(*mut Object, SEL) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s)
}
/// One-`NSInteger`-argument message that returns `BOOL` (e.g.
/// `setActivationPolicy:`) — its own wrapper rather than a `void`-returning
/// one with the result discarded, since discarding through a transmuted
/// function-pointer type that doesn't even declare the real return value is
/// itself a mistyped-function-pointer call (UB per Rust's FFI contract, even
/// though it happens to be harmless on arm64's calling convention for this
/// specific case).
pub(crate) unsafe fn send1_int_bool(recv: *mut Object, s: SEL, arg: NsInteger) -> ObjcBool {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsInteger) -> ObjcBool =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send1_obj(recv: *mut Object, s: SEL, arg: *mut Object) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send1_ptr_ret(recv: *mut Object, s: SEL, arg: *const c_char) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, *const c_char) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
/// One-`BOOL`-argument, `void`-returning message (e.g.
/// `activateIgnoringOtherApps:`).
pub(crate) unsafe fn send1_bool_void(recv: *mut Object, s: SEL, arg: ObjcBool) {
    let f: unsafe extern "C" fn(*mut Object, SEL, ObjcBool) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send1_uint_void(recv: *mut Object, s: SEL, arg: NsUInteger) {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send1_uint_obj(recv: *mut Object, s: SEL, arg: NsUInteger) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsUInteger) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send1_obj_obj(recv: *mut Object, s: SEL, arg: *mut Object) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send1_size_void(recv: *mut Object, s: SEL, arg: NsSize) {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsSize) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send1_f64_void(recv: *mut Object, s: SEL, arg: f64) {
    let f: unsafe extern "C" fn(*mut Object, SEL, f64) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
pub(crate) unsafe fn send2_obj_void(recv: *mut Object, s: SEL, a: *mut Object, b: *mut Object) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object, *mut Object) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a, b)
}
pub(crate) unsafe fn send2_obj_uint_void(recv: *mut Object, s: SEL, a: *mut Object, b: NsUInteger) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a, b)
}
