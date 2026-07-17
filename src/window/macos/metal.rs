//! Metal backend for macOS, additive alongside `gl.rs` (OpenGL/CGL) — never
//! a replacement (see `CLAUDE.md`'s standing Metal exception). Composes the
//! same [`CocoaWindowState`] as `gl.rs` and adds only the Metal-specific
//! pieces around it: a device, a command queue, a `CAMetalLayer` hosted by
//! the window's content view, and an app-owned offscreen render target.
//!
//! **Phase 1 (this commit): device/queue/layer plumbing; `clear` +
//! `swap_buffers` work end to end.** The `gfx.*` draw-call surface (MSL
//! shader compilation, buffers, the VAO shim, textures, uniforms, draws,
//! `read_pixels`) is Phase 2 — `mod.rs`'s forwarding arms for those panic
//! with a clear message until then.
//!
//! # Rendering model: offscreen target, blit at present
//!
//! `clear` (and, in Phase 2, every draw) renders into an app-owned offscreen
//! `MTLTexture`, not the drawable's own texture; `swap_buffers` acquires the
//! frame's `CAMetalDrawable`, blit-copies the offscreen target into it,
//! presents, and commits. Two reasons this indirection is load-bearing
//! rather than overhead:
//! - Drawable textures are transient — each `nextDrawable` hands back a
//!   different texture from a small internal pool, so "the back buffer" has
//!   no stable identity across calls the way a GL default framebuffer does.
//!   The offscreen target *is* the stable back buffer, letting `clear` /
//!   future draws happen at any point before `swap_buffers`, exactly like
//!   the GL call pattern demos already use.
//! - Phase 2's `read_pixels` needs a CPU-readable texture. The offscreen
//!   target uses `MTLStorageModeShared` (uniform memory on Apple Silicon —
//!   the only macOS hardware this backend compiles for, so no
//!   `Managed`-mode sync-blit dance is ever needed), while drawable
//!   textures are display-owned and not reliably readable.
//!
//! The layer's `drawableSize` is deliberately kept 1:1 with the window's
//! *point* size (`contentsScale` stays at its default 1.0), matching the GL
//! backend's non-Retina-scaled backing, so `width()`/`height()` and Phase
//! 2's `read_pixels` coordinates mean the same pixels on both backends.
//!
//! # Memory management: per-frame autorelease pools (a deliberate deviation)
//!
//! `shared.rs` uses one process-lifetime autorelease pool, justified by "no
//! autoreleased object is created more than a small constant number of
//! times per `poll()`". That justification does **not** transfer to this
//! file's frame path: `nextDrawable` hands back one of a small fixed pool
//! (~3) of drawables that the layer only reclaims when the autoreleased
//! drawable object is actually *released* — without a per-frame pool drain,
//! the third `swap_buffers` would block forever waiting for a drawable that
//! can never come back. So `create`/`clear`/`swap_buffers` each push/pop
//! their own [`AutoreleasePool`], draining every autoreleased frame object
//! (drawable, command buffer, pass descriptor, encoders) promptly.

use std::ffi::c_void;

use super::shared::{
    class, is_main_thread, objc_msgSend, sel, send0, send0_void, send1_bool_void, send1_obj,
    CocoaWindowState, NsSize, NsUInteger, Object, ObjcBool, OBJC_NO, OBJC_YES, SEL,
};

#[link(name = "Metal", kind = "framework")]
extern "C" {
    /// Returns a +1 reference (Create rule) or nil when no Metal-capable GPU
    /// exists in this environment (some headless VMs).
    fn MTLCreateSystemDefaultDevice() -> *mut Object;
}

// `CAMetalLayer` lives in QuartzCore, not Metal — linking the framework is
// what makes `objc_getClass("CAMetalLayer")` resolve at runtime.
#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

#[link(name = "objc")]
extern "C" {
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

/// RAII autorelease pool — see the module doc comment on why the frame path
/// needs per-call pools when `shared.rs` gets away without them.
struct AutoreleasePool(*mut c_void);
impl AutoreleasePool {
    fn push() -> AutoreleasePool {
        AutoreleasePool(unsafe { objc_autoreleasePoolPush() })
    }
}
impl Drop for AutoreleasePool {
    fn drop(&mut self) {
        unsafe { objc_autoreleasePoolPop(self.0) };
    }
}

/// `MTLClearColor`: four `f64`s. On arm64 a 4-double aggregate is a
/// homogeneous floating-point aggregate passed directly in `v0`-`v3` — no
/// `_stret`-style special-casing exists or is needed (same reasoning as
/// `NsRect` in `shared.rs`).
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlClearColor {
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
}

// Metal enum values (stable since Metal 1.0; same cross-corroboration
// discipline as shared.rs's AppKit constants).
const MTL_PIXEL_FORMAT_BGRA8_UNORM: NsUInteger = 80;
const MTL_LOAD_ACTION_CLEAR: NsUInteger = 2;
const MTL_STORE_ACTION_STORE: NsUInteger = 1;
const MTL_TEXTURE_USAGE_SHADER_READ: NsUInteger = 1 << 0;
const MTL_TEXTURE_USAGE_RENDER_TARGET: NsUInteger = 1 << 2;
/// CPU+GPU uniform memory — always valid on Apple Silicon (the only macOS
/// hardware this backend compiles for), which is what lets Phase 2's
/// `read_pixels` map the offscreen target directly.
const MTL_STORAGE_MODE_SHARED: NsUInteger = 0;

// Metal-shaped `objc_msgSend` wrappers this file needs beyond `shared.rs`'s
// set — same one-named-wrapper-per-call-shape discipline (see `shared.rs`'s
// doc comment on why that beats a variadic-emulating macro here).
unsafe fn send1_uint_void(recv: *mut Object, s: SEL, arg: NsUInteger) {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send1_uint_obj(recv: *mut Object, s: SEL, arg: NsUInteger) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsUInteger) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send1_obj_obj(recv: *mut Object, s: SEL, arg: *mut Object) -> *mut Object {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object) -> *mut Object =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send1_size_void(recv: *mut Object, s: SEL, arg: NsSize) {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsSize) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send1_clear_color_void(recv: *mut Object, s: SEL, arg: MtlClearColor) {
    let f: unsafe extern "C" fn(*mut Object, SEL, MtlClearColor) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send2_obj_void(recv: *mut Object, s: SEL, a: *mut Object, b: *mut Object) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut Object, *mut Object) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, a, b)
}
unsafe fn send_texture_descriptor(
    recv: *mut Object,
    s: SEL,
    format: NsUInteger,
    w: NsUInteger,
    h: NsUInteger,
    mipmapped: ObjcBool,
) -> *mut Object {
    let f: unsafe extern "C" fn(
        *mut Object,
        SEL,
        NsUInteger,
        NsUInteger,
        NsUInteger,
        ObjcBool,
    ) -> *mut Object = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, format, w, h, mipmapped)
}

/// Build the offscreen BGRA8 render target (see the module doc comment for
/// why it exists at all). `RenderTarget` usage for `clear`/Phase-2 draws,
/// `ShaderRead` + `Shared` storage for Phase-2 `read_pixels`. The descriptor
/// is autoreleased (class-method constructor) — every caller runs inside an
/// [`AutoreleasePool`].
unsafe fn new_render_target(device: *mut Object, w: i32, h: i32) -> Result<*mut Object, String> {
    let desc = send_texture_descriptor(
        class("MTLTextureDescriptor"),
        sel("texture2DDescriptorWithPixelFormat:width:height:mipmapped:"),
        MTL_PIXEL_FORMAT_BGRA8_UNORM,
        w as NsUInteger,
        h as NsUInteger,
        OBJC_NO,
    );
    if desc.is_null() {
        return Err(
            "window.create_metal: [MTLTextureDescriptor texture2DDescriptorWithPixelFormat:...] \
             returned nil"
                .to_string(),
        );
    }
    send1_uint_void(
        desc,
        sel("setUsage:"),
        MTL_TEXTURE_USAGE_RENDER_TARGET | MTL_TEXTURE_USAGE_SHADER_READ,
    );
    send1_uint_void(desc, sel("setStorageMode:"), MTL_STORAGE_MODE_SHARED);
    // `new...` prefix: +1 reference, released in `teardown`/on resize.
    let tex = send1_obj_obj(device, sel("newTextureWithDescriptor:"), desc);
    if tex.is_null() {
        return Err(
            "window.create_metal: [MTLDevice newTextureWithDescriptor:] returned nil".to_string(),
        );
    }
    Ok(tex)
}

pub struct Inner {
    cocoa: CocoaWindowState,
    /// +1 from `MTLCreateSystemDefaultDevice` (Create rule); released in
    /// `teardown`.
    device: *mut Object,
    /// +1 from `newCommandQueue`; released in `teardown`.
    queue: *mut Object,
    /// +1 from `alloc`+`init` (the content view holds its own reference once
    /// `setLayer:` runs); ours released in `teardown`.
    layer: *mut Object,
    /// The offscreen render target — see the module doc comment. +1 from
    /// `newTextureWithDescriptor:`; released in `teardown` and whenever a
    /// resize recreates it.
    target: *mut Object,
    /// Dimensions `target` and the layer's `drawableSize` were last created
    /// at, so [`Inner::resize_if_needed`] keeps the two in lockstep (the
    /// whole-texture blit in `swap_buffers` requires matching sizes).
    target_w: i32,
    target_h: i32,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        if !is_main_thread() {
            return Err(
                "window.create_metal: must be called from the process's main thread (AppKit \
                 requires NSWindow creation there; calling from a worker isolate is not \
                 supported)"
                    .to_string(),
            );
        }
        let _pool = AutoreleasePool::push();
        // Safety: standard minimal Metal-on-Cocoa setup recipe (the direct
        // analog of `gl.rs::create`'s pixel-format/context half); every
        // fallible step (a nil return) is checked, and each failure path
        // releases exactly what was created before it, mirroring `gl.rs`'s
        // "no partial resource leaks on a fallible step" discipline.
        unsafe {
            // Device before window: a headless environment with no Metal GPU
            // fails cleanly here without ever flashing a window.
            let device = MTLCreateSystemDefaultDevice();
            if device.is_null() {
                return Err(
                    "window.create_metal: MTLCreateSystemDefaultDevice returned nil (no \
                     Metal-capable GPU in this environment)"
                        .to_string(),
                );
            }
            let queue = send0(device, sel("newCommandQueue"));
            if queue.is_null() {
                send0_void(device, sel("release"));
                return Err(
                    "window.create_metal: [MTLDevice newCommandQueue] returned nil".to_string(),
                );
            }

            let cocoa = match CocoaWindowState::create_window(title, w, h) {
                Ok(c) => c,
                Err(e) => {
                    send0_void(queue, sel("release"));
                    send0_void(device, sel("release"));
                    return Err(e);
                }
            };

            // `alloc`+`init` rather than the `[CAMetalLayer layer]`
            // convenience constructor: the latter is autoreleased and would
            // die with this function's pool.
            let layer_alloc = send0(class("CAMetalLayer"), sel("alloc"));
            let layer = if layer_alloc.is_null() {
                std::ptr::null_mut()
            } else {
                send0(layer_alloc, sel("init"))
            };
            if layer.is_null() {
                cocoa.teardown();
                send0_void(queue, sel("release"));
                send0_void(device, sel("release"));
                return Err(
                    "window.create_metal: [[CAMetalLayer alloc] init] returned nil".to_string(),
                );
            }
            send1_obj(layer, sel("setDevice:"), device);
            send1_uint_void(layer, sel("setPixelFormat:"), MTL_PIXEL_FORMAT_BGRA8_UNORM);
            // framebufferOnly=YES (the default) forbids using drawable
            // textures as blit destinations — the offscreen-target design
            // (module doc comment) blits into them every frame.
            send1_bool_void(layer, sel("setFramebufferOnly:"), OBJC_NO);
            send1_size_void(
                layer,
                sel("setDrawableSize:"),
                NsSize {
                    width: w as f64,
                    height: h as f64,
                },
            );

            // Layer-hosting order matters: assign the custom layer FIRST,
            // then `setWantsLayer:YES`. The reverse order makes the view
            // layer-*backed* (AppKit creates and owns its own backing layer)
            // and the later `setLayer:` swap is less predictable mid-flight.
            let view = cocoa.content_view();
            send1_obj(view, sel("setLayer:"), layer);
            send1_bool_void(view, sel("setWantsLayer:"), OBJC_YES);

            let target = match new_render_target(device, w, h) {
                Ok(t) => t,
                Err(e) => {
                    send0_void(layer, sel("release"));
                    cocoa.teardown();
                    send0_void(queue, sel("release"));
                    send0_void(device, sel("release"));
                    return Err(e);
                }
            };

            cocoa.show();
            Ok(Inner {
                cocoa,
                device,
                queue,
                layer,
                target,
                target_w: w,
                target_h: h,
            })
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

    /// Metal has no GL-style per-thread "current context" to bind — the
    /// VM-level `gfx_current_window` slot (set by `win.make_current()`,
    /// see `natives.rs`) is the only state that concept needs, so at the
    /// backend level this is deliberately a no-op, not a stub awaiting
    /// implementation.
    pub fn make_current(&mut self) {}

    /// Encodes a render pass whose only work is the clear itself
    /// (`loadAction=Clear`, zero draws, `storeAction=Store`) into the
    /// offscreen target, and commits it — the queue serializes it ahead of
    /// whatever `swap_buffers` encodes next, so no explicit wait is needed
    /// here.
    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        let _pool = AutoreleasePool::push();
        unsafe {
            self.resize_if_needed();
            let cmd = send0(self.queue, sel("commandBuffer"));
            if cmd.is_null() {
                return;
            }
            let rpd = send0(class("MTLRenderPassDescriptor"), sel("renderPassDescriptor"));
            if rpd.is_null() {
                return;
            }
            let atts = send0(rpd, sel("colorAttachments"));
            let att = send1_uint_obj(atts, sel("objectAtIndexedSubscript:"), 0);
            send1_obj(att, sel("setTexture:"), self.target);
            send1_uint_void(att, sel("setLoadAction:"), MTL_LOAD_ACTION_CLEAR);
            send1_uint_void(att, sel("setStoreAction:"), MTL_STORE_ACTION_STORE);
            send1_clear_color_void(
                att,
                sel("setClearColor:"),
                MtlClearColor {
                    red: r as f64,
                    green: g as f64,
                    blue: b as f64,
                    alpha: a as f64,
                },
            );
            let enc = send1_obj_obj(cmd, sel("renderCommandEncoderWithDescriptor:"), rpd);
            if !enc.is_null() {
                send0_void(enc, sel("endEncoding"));
            }
            send0_void(cmd, sel("commit"));
        }
    }

    /// Present: blit the offscreen target into this frame's drawable,
    /// present it, commit. Waits for completion so the call is synchronous
    /// like a GL buffer swap — deterministic for golden tests, and it bounds
    /// in-flight work to one frame.
    pub fn swap_buffers(&mut self) {
        let _pool = AutoreleasePool::push();
        unsafe {
            self.resize_if_needed();
            // nil when the layer is zero-sized or the drawable pool is
            // exhausted — skip the frame rather than crash, matching how
            // `poll`-driven demos tolerate a minimized window.
            let drawable = send0(self.layer, sel("nextDrawable"));
            if drawable.is_null() {
                return;
            }
            let dtex = send0(drawable, sel("texture"));
            let cmd = send0(self.queue, sel("commandBuffer"));
            if dtex.is_null() || cmd.is_null() {
                return;
            }
            let blit = send0(cmd, sel("blitCommandEncoder"));
            if !blit.is_null() {
                // Whole-texture variant (macOS 10.15+, well below the Apple
                // Silicon 11.0 floor): formats and dimensions match by
                // construction — `resize_if_needed` keeps `target` and
                // `drawableSize` in lockstep.
                send2_obj_void(blit, sel("copyFromTexture:toTexture:"), self.target, dtex);
                send0_void(blit, sel("endEncoding"));
            }
            send1_obj(cmd, sel("presentDrawable:"), drawable);
            send0_void(cmd, sel("commit"));
            send0_void(cmd, sel("waitUntilCompleted"));
        }
    }

    /// Recreate the offscreen target and the layer's `drawableSize` when the
    /// window has been live-resized (`poll` keeps `cocoa.width/height`
    /// current), so the whole-texture blit's size/format precondition keeps
    /// holding. The first frame after a resize starts from a fresh (blank)
    /// target — the same single-frame artifact GL's resize has.
    unsafe fn resize_if_needed(&mut self) {
        let w = self.cocoa.width.max(1);
        let h = self.cocoa.height.max(1);
        if w == self.target_w && h == self.target_h {
            return;
        }
        if let Ok(t) = new_render_target(self.device, w, h) {
            send0_void(self.target, sel("release"));
            self.target = t;
            self.target_w = w;
            self.target_h = h;
            send1_size_void(
                self.layer,
                sel("setDrawableSize:"),
                NsSize {
                    width: w as f64,
                    height: h as f64,
                },
            );
        }
    }

    pub fn teardown(self) {
        unsafe {
            send0_void(self.target, sel("release"));
            send0_void(self.layer, sel("release"));
            send0_void(self.queue, sel("release"));
            send0_void(self.device, sel("release"));
        }
        self.cocoa.teardown();
    }
}
