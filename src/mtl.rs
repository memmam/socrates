//! Raw-FFI Metal primitives shared by the two Metal consumers — the
//! `window/macos/metal.rs` graphics backend and `gpu.rs`'s native compute
//! path — extracted (like `objc.rs`) when the second consumer arrived, per
//! PROJECT.md's shared-core rule. Everything specific to one consumer
//! (pixel formats, render-pass plumbing, `CAMetalLayer`) stays with that
//! consumer; this file holds only what both genuinely share: the device
//! constructor, buffer creation, and MSL library compilation.

use std::ffi::c_void;

use crate::objc::{
    ns_string, nsstring_to_owned, objc_msgSend, sel, send0, NsUInteger, Object, SEL,
};

#[link(name = "Metal", kind = "framework")]
extern "C" {
    /// Returns a +1 reference (Create rule) or nil when no Metal-capable GPU
    /// exists in this environment (some headless VMs).
    pub(crate) fn MTLCreateSystemDefaultDevice() -> *mut Object;
}

/// `MTLResourceOptions` with storage-mode Shared (CPU+GPU uniform memory —
/// always valid on Apple Silicon, the only macOS hardware this crate
/// targets) and default CPU cache mode — the plain `0` every
/// `newBufferWith...` call wants.
pub(crate) const MTL_RESOURCE_STORAGE_MODE_SHARED: NsUInteger = 0;

/// `MTLSize` — three `NSUInteger`s. Passed by value (indirectly, per the
/// arm64 ABI for >16-byte aggregates, which the correctly-typed transmute
/// handles).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct MtlSize {
    pub(crate) width: NsUInteger,
    pub(crate) height: NsUInteger,
    pub(crate) depth: NsUInteger,
}

/// `setBuffer:offset:atIndex:` (compute command encoder).
pub(crate) unsafe fn send_set_buffer(
    recv: *mut Object,
    s: SEL,
    buf: *mut Object,
    offset: NsUInteger,
    index: NsUInteger,
) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object, NsUInteger, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, buf, offset, index)
}

/// `dispatchThreadgroups:threadsPerThreadgroup:` — two by-value `MTLSize`s.
pub(crate) unsafe fn send_dispatch_threadgroups(
    recv: *mut Object,
    s: SEL,
    groups: MtlSize,
    per_group: MtlSize,
) {
    let f: unsafe extern "C" fn(*mut Object, SEL, MtlSize, MtlSize) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, groups, per_group)
}

/// `newComputePipelineStateWithFunction:error:` — +1 result.
pub(crate) unsafe fn send_new_compute_pipeline(
    recv: *mut Object,
    s: SEL,
    fun: *mut Object,
    error: *mut *mut Object,
) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object, *mut *mut Object) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, fun, error)
}

/// `newBufferWithBytes:length:options:` — +1 result.
pub(crate) unsafe fn send_new_buffer(
    recv: *mut Object,
    s: SEL,
    bytes: *const c_void,
    len: NsUInteger,
    options: NsUInteger,
) -> *mut Object {
    let f: unsafe extern "C" fn(
        *mut Object,
        SEL,
        *const c_void,
        NsUInteger,
        NsUInteger,
    ) -> *mut Object = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, bytes, len, options)
}

/// `newBufferWithLength:options:` — +1 result; for zero-byte-input edge
/// cases (`newBufferWithBytes` rejects a zero length). Contents are NOT
/// guaranteed zeroed — callers that need zeroing write it themselves via
/// `contents`.
pub(crate) unsafe fn send_new_buffer_len(
    recv: *mut Object,
    s: SEL,
    len: NsUInteger,
    options: NsUInteger,
) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsUInteger, NsUInteger) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, len, options)
}

/// `newLibraryWithSource:options:error:` — the raw shape behind
/// [`new_library`].
unsafe fn send_new_library(
    recv: *mut Object,
    s: SEL,
    src: *mut Object,
    options: *mut Object,
    error: *mut *mut Object,
) -> *mut Object {
    let f: unsafe extern "C" fn(
        *mut Object,
        SEL,
        *mut Object,
        *mut Object,
        *mut *mut Object,
    ) -> *mut Object = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, src, options, error)
}

/// Compile one MSL source into an `MTLLibrary` (+1), turning the `NSError`
/// into the same kind of driver-log `Err` text GL's compile path produces.
/// Callers must hold an [`crate::objc::AutoreleasePool`] (the source
/// `NSString` and any `NSError` are autoreleased).
pub(crate) unsafe fn new_library(device: *mut Object, src: &str) -> Result<*mut Object, String> {
    let mut err: *mut Object = std::ptr::null_mut();
    let lib = send_new_library(
        device,
        sel("newLibraryWithSource:options:error:"),
        ns_string(src),
        std::ptr::null_mut(),
        &mut err,
    );
    if lib.is_null() {
        let msg = if err.is_null() {
            "unknown MSL compile error".to_string()
        } else {
            nsstring_to_owned(send0(err, sel("localizedDescription")))
        };
        return Err(msg);
    }
    Ok(lib)
}
