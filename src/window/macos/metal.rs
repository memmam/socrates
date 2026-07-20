//! Metal backend for macOS, additive alongside `gl.rs` (OpenGL/CGL) — never
//! a replacement (see `PROJECT.md`'s standing Metal exception). Composes the
//! same [`CocoaWindowState`] as `gl.rs` and adds only the Metal-specific
//! pieces around it: a device, a command queue, a `CAMetalLayer` hosted by
//! the window's content view, an app-owned offscreen render target, and —
//! as of Phase 2 — the full `gfx.*` draw-call surface.
//!
//! # The `gfx` surface on Metal: conventions and mappings
//!
//! `gfx.*` was specified against OpenGL 3.3 (SPEC § 7.4); this backend gives
//! every call the same *observable* semantics with per-API differences
//! confined to shader source text (the one escape hatch, per
//! `win.backend_name()`):
//!
//! - **Shaders are MSL, entry points fixed**: `compile_program`'s two
//!   sources are each a standalone MSL translation unit whose entry
//!   functions are named `vertex_main` and `fragment_main` respectively —
//!   the direct analog of GLSL's fixed `main` per stage.
//! - **Uniforms**: each stage's uniforms live in one struct argument bound
//!   at `[[buffer(0)]]`. `set_uniform_*(program, name, v)` stages the bytes
//!   CPU-side; at draw time the member's byte offset is resolved by *name*
//!   from pipeline reflection (`MTLRenderPipelineReflection` →
//!   `bufferStructType` → `MTLStructMember.offset`) and the staged struct is
//!   uploaded with `setVertexBytes:`/`setFragmentBytes:` — mirroring
//!   `glGetUniformLocation` + `glUniform*`, including silently ignoring
//!   names the shader doesn't declare. (Every `gfx` uniform is far below
//!   Metal's 4 KiB inline-constant limit, so no separate `MTLBuffer` is
//!   ever needed for uniforms.)
//! - **Vertex attributes**: Metal has no VAO concept, so this file keeps a
//!   Rust-side record per VAO id — `(index → size/stride/offset/buffer)`
//!   captured by `set_vertex_attrib` exactly like GL captures the bound
//!   `GL_ARRAY_BUFFER` into VAO state — replayed at draw time as an
//!   `MTLVertexDescriptor` plus `setVertexBuffer:offset:atIndex:` calls.
//!   Attribute `i`'s backing buffer is bound at index `1 + i` (index 0 is
//!   reserved for the uniform struct), so MSL vertex shaders use
//!   `[[stage_in]]` with `[[attribute(i)]]` and never name buffer indices.
//! - **Textures**: `MTLTexture` handles in a table (Metal objects are
//!   pointers, not driver-issued integers, so GL-style `Int` handles need
//!   the `u32 → retained pointer` map); bound per unit and set on the
//!   encoder as `[[texture(unit)]]`. GL's `upload_texture` sampler state
//!   (linear, clamp-to-edge) has no API-object equivalent here — MSL
//!   shaders declare their own `constexpr sampler`, which is the idiomatic
//!   Metal spelling of the same fixed sampling mode.
//! - **Pipelines**: GL binds program and vertex layout independently; Metal
//!   fuses them into one `MTLRenderPipelineState`. Built lazily at draw
//!   time and cached per `(program, vertex-layout fingerprint)` — pipeline
//!   compilation is expensive, draws are not.
//! - **One render encoder per draw**: GL interleaves state changes and
//!   draws freely against a persistent framebuffer; the loss-free mapping
//!   is a fresh `loadAction=Load` render pass per draw call into the
//!   persistent offscreen target. Wasteful for huge draw counts, but
//!   observably identical — and per PROJECT.md's efficiency-pass rule, the
//!   faster batched-encoder idiom can later become the primitive underneath
//!   this exact surface without changing a single pinned byte.
//! - **Y origin**: GL is bottom-left, Metal top-left. `viewport` and
//!   `read_pixels` flip `y` internally (`metal_y = target_h - y - h`), and
//!   `read_pixels` additionally returns rows bottom-up and RGBA-ordered
//!   (swizzled from the BGRA target), so **the same call means the same
//!   physical pixels on both backends** — demos/glcube's corner-pixel pins
//!   must not silently read the wrong corner.
//!
//! # Rendering model: offscreen target, blit at present (Phase 1)
//!
//! All rendering lands in an app-owned offscreen `MTLTexture` (plus a
//! matching `Depth32Float` texture); `swap_buffers` acquires the frame's
//! `CAMetalDrawable`, blit-copies the color target into it, presents, and
//! commits. Drawable textures are transient (no stable back-buffer identity
//! across `nextDrawable` calls) and not reliably CPU-readable; the offscreen
//! target is both the stable GL-style back buffer and — being
//! `MTLStorageModeShared` (uniform memory on Apple Silicon, the only macOS
//! hardware this backend compiles for) — the thing `read_pixels` maps
//! directly with no sync-blit dance.
//!
//! The layer's `drawableSize` is deliberately kept 1:1 with the window's
//! *point* size (`contentsScale` stays 1.0), matching the GL backend's
//! non-Retina-scaled backing, so `width()`/`height()`/`read_pixels`
//! coordinates mean the same pixels on both backends.
//!
//! # Memory management: per-frame autorelease pools (a deliberate deviation)
//!
//! `shared.rs` uses one process-lifetime autorelease pool, justified by "no
//! autoreleased object is created more than a small constant number of
//! times per `poll()`". That justification does **not** transfer to this
//! file's frame path: `nextDrawable` hands back one of a small fixed pool
//! (~3) of drawables that the layer only reclaims when the autoreleased
//! drawable object is actually *released* — without a per-frame pool drain,
//! the third `swap_buffers` would block forever. So every method that
//! creates autoreleased objects (frame ops, draws, shader compiles, texture
//! uploads, readbacks) pushes/pops its own [`AutoreleasePool`].

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::c_void;

use super::shared::{is_main_thread, CocoaWindowState};
use crate::mtl::{
    new_library, send_new_buffer, send_new_buffer_len, MTLCreateSystemDefaultDevice,
    MTL_RESOURCE_STORAGE_MODE_SHARED,
};
use crate::objc::{
    class, ns_string, nsstring_to_owned, objc_msgSend, sel, send0, send0_uint, send0_void,
    send1_bool_void, send1_f64_void, send1_obj, send1_obj_obj, send1_size_void, send1_uint_obj,
    send1_uint_void, send2_obj_uint_void, send2_obj_void, AutoreleasePool, NsSize, NsUInteger,
    Object, ObjcBool, OBJC_NO, OBJC_YES, SEL,
};

// `CAMetalLayer` lives in QuartzCore, not Metal — linking the framework is
// what makes `objc_getClass("CAMetalLayer")` resolve at runtime.
#[link(name = "QuartzCore", kind = "framework")]
extern "C" {}

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

/// `MTLViewport`: six `f64`s (48 bytes — passed indirectly per the arm64
/// ABI, which the correctly-typed transmute handles).
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlViewport {
    origin_x: f64,
    origin_y: f64,
    width: f64,
    height: f64,
    znear: f64,
    zfar: f64,
}

/// `MTLRegion` = `MTLOrigin{x,y,z}` + `MTLSize{width,height,depth}` — six
/// `NSUInteger`s, laid out flat here (identical memory layout).
#[repr(C)]
#[derive(Clone, Copy)]
struct MtlRegion {
    x: NsUInteger,
    y: NsUInteger,
    z: NsUInteger,
    w: NsUInteger,
    h: NsUInteger,
    d: NsUInteger,
}

// Metal enum values (stable since Metal 1.0; same cross-corroboration
// discipline as shared.rs's AppKit constants).
const MTL_PIXEL_FORMAT_RGBA8_UNORM: NsUInteger = 70;
const MTL_PIXEL_FORMAT_BGRA8_UNORM: NsUInteger = 80;
const MTL_PIXEL_FORMAT_DEPTH32_FLOAT: NsUInteger = 252;
const MTL_LOAD_ACTION_LOAD: NsUInteger = 1;
const MTL_LOAD_ACTION_CLEAR: NsUInteger = 2;
const MTL_STORE_ACTION_STORE: NsUInteger = 1;
const MTL_TEXTURE_USAGE_SHADER_READ: NsUInteger = 1 << 0;
const MTL_TEXTURE_USAGE_RENDER_TARGET: NsUInteger = 1 << 2;
/// CPU+GPU uniform memory — always valid on Apple Silicon, which is what
/// lets `read_pixels` map the offscreen target directly.
const MTL_STORAGE_MODE_SHARED: NsUInteger = 0;
/// GPU-only — the depth texture is never read back.
const MTL_STORAGE_MODE_PRIVATE: NsUInteger = 2;
const MTL_PRIMITIVE_TYPE_TRIANGLE: NsUInteger = 3;
/// `gfx.draw_elements` indices are 32-bit on every backend (`gl.rs` uses
/// `GL_UNSIGNED_INT`).
const MTL_INDEX_TYPE_UINT32: NsUInteger = 1;
/// `MTLVertexFormatFloat` — sizes 1..=4 map to `28 + (size - 1)`
/// (float/float2/float3/float4), matching `set_vertex_attrib`'s GL_FLOAT-only
/// contract.
const MTL_VERTEX_FORMAT_FLOAT: NsUInteger = 28;
const MTL_COMPARE_FUNCTION_LESS: NsUInteger = 1;
const MTL_COMPARE_FUNCTION_ALWAYS: NsUInteger = 7;
const MTL_ARGUMENT_TYPE_BUFFER: NsUInteger = 0;
/// `MTLPipelineOptionArgumentInfo | MTLPipelineOptionBufferTypeInfo` — asks
/// pipeline creation to also produce the reflection object the uniform
/// name→offset resolution reads.
const MTL_PIPELINE_OPTION_REFLECTION: NsUInteger = (1 << 0) | (1 << 1);

// Metal-shaped `objc_msgSend` wrappers beyond `shared.rs`'s set — same
// one-named-wrapper-per-call-shape discipline (see `shared.rs`'s doc comment
// on why that beats a variadic-emulating macro here).
unsafe fn send1_clear_color_void(recv: *mut Object, s: SEL, arg: MtlClearColor) {
    let f: unsafe extern "C" fn(*mut Object, SEL, MtlClearColor) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
}
unsafe fn send1_viewport_void(recv: *mut Object, s: SEL, arg: MtlViewport) {
    let f: unsafe extern "C" fn(*mut Object, SEL, MtlViewport) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, arg)
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
/// `newRenderPipelineStateWithDescriptor:options:reflection:error:`.
unsafe fn send_new_pipeline(
    recv: *mut Object,
    s: SEL,
    desc: *mut Object,
    options: NsUInteger,
    reflection: *mut *mut Object,
    error: *mut *mut Object,
) -> *mut Object {
    let f: unsafe extern "C" fn(
        *mut Object,
        SEL,
        *mut Object,
        NsUInteger,
        *mut *mut Object,
        *mut *mut Object,
    ) -> *mut Object = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, desc, options, reflection, error)
}
/// `setVertexBuffer:offset:atIndex:`.
unsafe fn send_set_vertex_buffer(
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
/// `setVertexBytes:length:atIndex:` / `setFragmentBytes:length:atIndex:`.
unsafe fn send_set_bytes(
    recv: *mut Object,
    s: SEL,
    bytes: *const c_void,
    len: NsUInteger,
    index: NsUInteger,
) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *const c_void, NsUInteger, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, bytes, len, index)
}
/// `drawPrimitives:vertexStart:vertexCount:`.
unsafe fn send_draw_primitives(
    recv: *mut Object,
    s: SEL,
    prim: NsUInteger,
    start: NsUInteger,
    count: NsUInteger,
) {
    let f: unsafe extern "C" fn(*mut Object, SEL, NsUInteger, NsUInteger, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, prim, start, count)
}
/// `drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:`.
unsafe fn send_draw_indexed(
    recv: *mut Object,
    s: SEL,
    prim: NsUInteger,
    count: NsUInteger,
    index_type: NsUInteger,
    index_buffer: *mut Object,
    offset: NsUInteger,
) {
    let f: unsafe extern "C" fn(
        *mut Object,
        SEL,
        NsUInteger,
        NsUInteger,
        NsUInteger,
        *mut Object,
        NsUInteger,
    ) = std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, prim, count, index_type, index_buffer, offset)
}
/// `replaceRegion:mipmapLevel:withBytes:bytesPerRow:`.
unsafe fn send_replace_region(
    recv: *mut Object,
    s: SEL,
    region: MtlRegion,
    level: NsUInteger,
    bytes: *const c_void,
    per_row: NsUInteger,
) {
    let f: unsafe extern "C" fn(*mut Object, SEL, MtlRegion, NsUInteger, *const c_void, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, region, level, bytes, per_row)
}
/// `getBytes:bytesPerRow:fromRegion:mipmapLevel:`.
unsafe fn send_get_bytes(
    recv: *mut Object,
    s: SEL,
    out: *mut c_void,
    per_row: NsUInteger,
    region: MtlRegion,
    level: NsUInteger,
) {
    let f: unsafe extern "C" fn(*mut Object, SEL, *mut c_void, NsUInteger, MtlRegion, NsUInteger) =
        std::mem::transmute(objc_msgSend as *const ());
    f(recv, s, out, per_row, region, level)
}

/// Build an offscreen texture. `RenderTarget` usage always;
/// color targets add `ShaderRead` + `Shared` storage (CPU-readable for
/// `read_pixels`), the depth target is `Private` (GPU-only).
unsafe fn new_offscreen_texture(
    device: *mut Object,
    format: NsUInteger,
    storage: NsUInteger,
    usage: NsUInteger,
    w: i32,
    h: i32,
) -> Result<*mut Object, String> {
    let desc = send_texture_descriptor(
        class("MTLTextureDescriptor"),
        sel("texture2DDescriptorWithPixelFormat:width:height:mipmapped:"),
        format,
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
    send1_uint_void(desc, sel("setUsage:"), usage);
    send1_uint_void(desc, sel("setStorageMode:"), storage);
    // `new...` prefix: +1 reference, released in `teardown`/on resize.
    let tex = send1_obj_obj(device, sel("newTextureWithDescriptor:"), desc);
    if tex.is_null() {
        return Err(
            "window.create_metal: [MTLDevice newTextureWithDescriptor:] returned nil".to_string(),
        );
    }
    Ok(tex)
}

/// One `set_vertex_attrib` record — what GL stores in VAO state, kept in
/// Rust instead since Metal has no VAO object at all. `buffer` is *our* u32
/// handle (not the `MTLBuffer` pointer): resolution happens at draw time, so
/// a later `upload_buffer` to the same handle is seen by existing VAOs,
/// matching `glBufferData`'s same-object-new-store semantics.
#[derive(Clone, Copy)]
struct Attrib {
    size: i32,
    stride: i32,
    offset: i32,
    buffer: u32,
}

/// A VAO record: attribute set plus the captured index-buffer binding
/// (`GL_ELEMENT_ARRAY_BUFFER` binding is VAO state in GL, mirrored here).
#[derive(Default)]
struct VaoRec {
    attribs: BTreeMap<u32, Attrib>,
    index_buffer: u32,
}

/// One stage's reflected uniform-struct layout: total byte size of the
/// `[[buffer(0)]]` argument and each member's name → byte offset.
struct StageLayout {
    size: usize,
    offsets: HashMap<String, usize>,
}

impl StageLayout {
    fn empty() -> StageLayout {
        StageLayout {
            size: 0,
            offsets: HashMap::new(),
        }
    }
}

/// A compiled program: the two `MTLFunction`s (+1 each), the staged uniform
/// values (name → raw bytes, last write wins — GL uniform state persists on
/// the program object the same way), the reflected per-stage layouts
/// (filled at first pipeline build), and the pipeline cache.
struct ProgramRec {
    vfun: *mut Object,
    ffun: *mut Object,
    uniforms: HashMap<String, Vec<u8>>,
    vertex_layout: Option<StageLayout>,
    fragment_layout: Option<StageLayout>,
    /// (vertex-layout fingerprint) → `MTLRenderPipelineState` (+1).
    psos: HashMap<u64, *mut Object>,
    /// Layouts whose pipeline build already failed — reported once via
    /// eprintln, then skipped instead of respammed every frame.
    failed_layouts: HashSet<u64>,
}

/// Fingerprint of the pipeline-relevant part of a vertex layout: the set of
/// `(index, size, effective stride)` — buffer identity and base offset are
/// binding-time state that doesn't shape the `MTLVertexDescriptor`. FNV-1a
/// fold; in-process cache key only, so stability across runs is irrelevant.
fn layout_fingerprint(attribs: &[(u32, Attrib)]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for (i, a) in attribs {
        for v in [
            *i as u64,
            a.size as u64,
            effective_stride(a) as u64,
        ] {
            h ^= v;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// GL treats `stride == 0` as "tightly packed" — resolve it to the real
/// byte stride before it reaches the fingerprint or the vertex descriptor.
fn effective_stride(a: &Attrib) -> i32 {
    if a.stride == 0 {
        a.size * 4
    } else {
        a.stride
    }
}

/// The real guts of the Metal `WindowHandle` variant — a
/// [`CocoaWindowState`] plus the device/queue/layer/offscreen-target
/// plumbing (Phase 1) and the `gfx` state tables (Phase 2).
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
    /// The offscreen color target — see the module doc comment. +1; released
    /// in `teardown` and whenever a resize recreates it.
    target: *mut Object,
    /// The matching offscreen depth target (`Depth32Float`, `Private`).
    depth: *mut Object,
    /// Dimensions `target`/`depth`/`drawableSize` were last created at, so
    /// [`Inner::resize_if_needed`] keeps all three in lockstep (the
    /// whole-texture blit in `swap_buffers` requires matching sizes).
    target_w: i32,
    target_h: i32,

    // ---- gfx state (Phase 2) ----
    programs: HashMap<u32, ProgramRec>,
    next_program: u32,
    /// u32 handle → `MTLBuffer` (+1). No entry until the first
    /// `upload_buffer`, mirroring GL's deferred data-store creation.
    buffers: HashMap<u32, *mut Object>,
    next_buffer: u32,
    vaos: HashMap<u32, VaoRec>,
    next_vao: u32,
    current_vao: u32,
    current_array_buffer: u32,
    /// u32 handle → `MTLTexture` (+1). No entry until `upload_texture`.
    textures: HashMap<u32, *mut Object>,
    next_texture: u32,
    active_unit: u32,
    /// unit → bound texture handle (`bind_texture` writes the active unit's
    /// slot, exactly like `glActiveTexture` + `glBindTexture`).
    unit_bindings: BTreeMap<u32, u32>,
    current_program: u32,
    depth_test: bool,
    /// The 2-state `MTLDepthStencilState` cache (Metal has no
    /// enable/disable toggle): `Always`+no-write ≙ GL depth test off,
    /// `Less`+write ≙ GL's default-func depth test on. Built lazily at
    /// first draw; +1 each; released in `teardown`.
    depth_state_off: *mut Object,
    depth_state_on: *mut Object,
    /// GL-coordinate viewport rect (bottom-left origin), y-flipped into an
    /// `MTLViewport` per encoder. `None` = full target (GL's initial state).
    viewport_rect: Option<(i32, i32, i32, i32)>,
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

            let target = match new_offscreen_texture(
                device,
                MTL_PIXEL_FORMAT_BGRA8_UNORM,
                MTL_STORAGE_MODE_SHARED,
                MTL_TEXTURE_USAGE_RENDER_TARGET | MTL_TEXTURE_USAGE_SHADER_READ,
                w,
                h,
            ) {
                Ok(t) => t,
                Err(e) => {
                    send0_void(layer, sel("release"));
                    cocoa.teardown();
                    send0_void(queue, sel("release"));
                    send0_void(device, sel("release"));
                    return Err(e);
                }
            };
            let depth = match new_offscreen_texture(
                device,
                MTL_PIXEL_FORMAT_DEPTH32_FLOAT,
                MTL_STORAGE_MODE_PRIVATE,
                MTL_TEXTURE_USAGE_RENDER_TARGET,
                w,
                h,
            ) {
                Ok(t) => t,
                Err(e) => {
                    send0_void(target, sel("release"));
                    send0_void(layer, sel("release"));
                    cocoa.teardown();
                    send0_void(queue, sel("release"));
                    send0_void(device, sel("release"));
                    return Err(e);
                }
            };

            cocoa.show();
            // VAO id 0 exists from the start: GL core profile technically
            // requires an explicit VAO, but being forgiving here (attribs
            // recorded against "no VAO" still work) costs one map entry and
            // removes a whole class of silent-no-draw surprises.
            let mut vaos = HashMap::new();
            vaos.insert(0, VaoRec::default());
            Ok(Inner {
                cocoa,
                device,
                queue,
                layer,
                target,
                depth,
                target_w: w,
                target_h: h,
                programs: HashMap::new(),
                next_program: 1,
                buffers: HashMap::new(),
                next_buffer: 1,
                vaos,
                next_vao: 1,
                current_vao: 0,
                current_array_buffer: 0,
                textures: HashMap::new(),
                next_texture: 1,
                active_unit: 0,
                unit_bindings: BTreeMap::new(),
                current_program: 0,
                depth_test: false,
                depth_state_off: std::ptr::null_mut(),
                depth_state_on: std::ptr::null_mut(),
                viewport_rect: None,
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

    /// `Window.clear` — color only (SPEC § 8.4d), unlike [`Inner::gfx_clear`]
    /// which also clears depth. Encodes a render pass whose only work is the
    /// clear itself into the offscreen target and commits it — the queue
    /// serializes it ahead of whatever comes next, so no explicit wait is
    /// needed here.
    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        let _pool = AutoreleasePool::push();
        unsafe {
            self.resize_if_needed();
            let cmd = send0(self.queue, sel("commandBuffer"));
            if cmd.is_null() {
                return;
            }
            let rpd = self.clear_pass_descriptor(r, g, b, a, false);
            if rpd.is_null() {
                return;
            }
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

    // -----------------------------------------------------------------
    // gfx.* (Phase 2) — see the module doc comment for the full mapping.
    // -----------------------------------------------------------------

    pub fn compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        let _pool = AutoreleasePool::push();
        // Safety: library/function creation with nil-checks on every
        // fallible step; each failure path releases exactly what was
        // created before it, mirroring gl.rs's compile_program cleanup.
        unsafe {
            let vlib = new_library(self.device, vertex_src)
                .map_err(|e| format!("vertex shader: {e}"))?;
            let vfun = send1_obj_obj(vlib, sel("newFunctionWithName:"), ns_string("vertex_main"));
            send0_void(vlib, sel("release")); // the function retains its library
            if vfun.is_null() {
                return Err("vertex shader: no function named `vertex_main` (the Metal \
                            backend's fixed entry-point convention — see SPEC § 7.4)"
                    .to_string());
            }
            let flib = match new_library(self.device, fragment_src) {
                Ok(l) => l,
                Err(e) => {
                    send0_void(vfun, sel("release"));
                    return Err(format!("fragment shader: {e}"));
                }
            };
            let ffun = send1_obj_obj(flib, sel("newFunctionWithName:"), ns_string("fragment_main"));
            send0_void(flib, sel("release"));
            if ffun.is_null() {
                send0_void(vfun, sel("release"));
                return Err("fragment shader: no function named `fragment_main` (the Metal \
                            backend's fixed entry-point convention — see SPEC § 7.4)"
                    .to_string());
            }
            let id = self.next_program;
            self.next_program += 1;
            self.programs.insert(
                id,
                ProgramRec {
                    vfun,
                    ffun,
                    uniforms: HashMap::new(),
                    vertex_layout: None,
                    fragment_layout: None,
                    psos: HashMap::new(),
                    failed_layouts: HashSet::new(),
                },
            );
            Ok(id)
        }
    }

    pub fn use_program(&mut self, program: u32) {
        self.current_program = program;
    }

    pub fn delete_program(&mut self, program: u32) {
        if let Some(rec) = self.programs.remove(&program) {
            unsafe {
                send0_void(rec.vfun, sel("release"));
                send0_void(rec.ffun, sel("release"));
                for (_, pso) in rec.psos {
                    send0_void(pso, sel("release"));
                }
            }
        }
        if self.current_program == program {
            self.current_program = 0;
        }
    }

    pub fn create_buffer(&mut self) -> u32 {
        let id = self.next_buffer;
        self.next_buffer += 1;
        id
    }

    pub fn delete_buffer(&mut self, buffer: u32) {
        if let Some(buf) = self.buffers.remove(&buffer) {
            unsafe { send0_void(buf, sel("release")) };
        }
    }

    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        match kind {
            crate::window::GfxBufferKind::Vertex => self.current_array_buffer = buffer,
            // The element-array binding is VAO state in GL — captured into
            // the current VAO record, exactly like set_vertex_attrib
            // captures the array-buffer binding.
            crate::window::GfxBufferKind::Index => {
                self.vaos.entry(self.current_vao).or_default().index_buffer = buffer;
            }
        }
    }

    pub fn upload_buffer(
        &mut self,
        kind: crate::window::GfxBufferKind,
        data: &[u8],
        _dynamic: bool,
    ) {
        // `dynamic` is a GL usage *hint*; Metal Shared-storage buffers have
        // no equivalent knob worth modeling — accepted and ignored.
        let id = match kind {
            crate::window::GfxBufferKind::Vertex => self.current_array_buffer,
            crate::window::GfxBufferKind::Index => {
                self.vaos.get(&self.current_vao).map_or(0, |v| v.index_buffer)
            }
        };
        if id == 0 {
            return;
        }
        let _pool = AutoreleasePool::push();
        unsafe {
            let new = if data.is_empty() {
                send_new_buffer_len(
                    self.device,
                    sel("newBufferWithLength:options:"),
                    1,
                    MTL_RESOURCE_STORAGE_MODE_SHARED,
                )
            } else {
                send_new_buffer(
                    self.device,
                    sel("newBufferWithBytes:length:options:"),
                    data.as_ptr() as *const c_void,
                    data.len() as NsUInteger,
                    MTL_RESOURCE_STORAGE_MODE_SHARED,
                )
            };
            if new.is_null() {
                return;
            }
            // glBufferData semantics: same handle, brand-new data store.
            if let Some(old) = self.buffers.insert(id, new) {
                send0_void(old, sel("release"));
            }
        }
    }

    pub fn create_vertex_array(&mut self) -> u32 {
        let id = self.next_vao;
        self.next_vao += 1;
        self.vaos.insert(id, VaoRec::default());
        id
    }

    pub fn bind_vertex_array(&mut self, vao: u32) {
        self.current_vao = vao;
        self.vaos.entry(vao).or_default();
    }

    pub fn delete_vertex_array(&mut self, vao: u32) {
        self.vaos.remove(&vao);
        if self.current_vao == vao {
            self.current_vao = 0;
        }
    }

    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        let buffer = self.current_array_buffer;
        self.vaos
            .entry(self.current_vao)
            .or_default()
            .attribs
            .insert(
                index,
                Attrib {
                    size,
                    stride,
                    offset,
                    buffer,
                },
            );
    }

    pub fn disable_vertex_attrib(&mut self, index: u32) {
        if let Some(rec) = self.vaos.get_mut(&self.current_vao) {
            rec.attribs.remove(&index);
        }
    }

    pub fn create_texture(&mut self) -> u32 {
        let id = self.next_texture;
        self.next_texture += 1;
        id
    }

    pub fn delete_texture(&mut self, tex: u32) {
        if let Some(t) = self.textures.remove(&tex) {
            unsafe { send0_void(t, sel("release")) };
        }
    }

    pub fn bind_texture(&mut self, tex: u32) {
        if tex == 0 {
            self.unit_bindings.remove(&self.active_unit);
        } else {
            self.unit_bindings.insert(self.active_unit, tex);
        }
    }

    pub fn active_texture_unit(&mut self, unit: u32) {
        self.active_unit = unit;
    }

    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        let Some(&id) = self.unit_bindings.get(&self.active_unit) else {
            return;
        };
        if width <= 0 || height <= 0 {
            return;
        }
        // Metal has no RGB8 format — expand 3-byte pixels to RGBA (alpha
        // 255) CPU-side. RGBA input uploads as-is.
        let (pixels, expanded);
        let bytes: &[u8] = if has_alpha {
            data
        } else {
            expanded = data
                .chunks_exact(3)
                .flat_map(|p| [p[0], p[1], p[2], 255u8])
                .collect::<Vec<u8>>();
            &expanded
        };
        pixels = bytes;
        let need = (width as usize) * (height as usize) * 4;
        if pixels.len() < need {
            // gfx.upload_texture's natives-layer length check (see PR #46)
            // already errors before reaching here; this is defense in depth
            // against an OOB read, not a reachable path.
            return;
        }
        let _pool = AutoreleasePool::push();
        unsafe {
            let tex = match new_offscreen_texture(
                self.device,
                MTL_PIXEL_FORMAT_RGBA8_UNORM,
                MTL_STORAGE_MODE_SHARED,
                MTL_TEXTURE_USAGE_SHADER_READ,
                width,
                height,
            ) {
                Ok(t) => t,
                Err(_) => return,
            };
            send_replace_region(
                tex,
                sel("replaceRegion:mipmapLevel:withBytes:bytesPerRow:"),
                MtlRegion {
                    x: 0,
                    y: 0,
                    z: 0,
                    w: width as NsUInteger,
                    h: height as NsUInteger,
                    d: 1,
                },
                0,
                pixels.as_ptr() as *const c_void,
                (width as NsUInteger) * 4,
            );
            if let Some(old) = self.textures.insert(id, tex) {
                send0_void(old, sel("release"));
            }
        }
    }

    fn stage_uniform(&mut self, program: u32, name: &str, bytes: Vec<u8>) {
        if let Some(rec) = self.programs.get_mut(&program) {
            rec.uniforms.insert(name.to_string(), bytes);
        }
    }

    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        self.stage_uniform(program, name, v.to_ne_bytes().to_vec());
    }

    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        self.stage_uniform(program, name, v.to_ne_bytes().to_vec());
    }

    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        let mut b = Vec::with_capacity(8);
        for v in [x, y] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        self.stage_uniform(program, name, b);
    }

    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        // MSL float3 occupies 16 bytes in a struct; writing 12 leaves the
        // padding untouched, which is exactly what GL does too.
        let mut b = Vec::with_capacity(12);
        for v in [x, y, z] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        self.stage_uniform(program, name, b);
    }

    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        let mut b = Vec::with_capacity(16);
        for v in [x, y, z, w] {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        self.stage_uniform(program, name, b);
    }

    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        // Column-major on both sides: GLSL mat4 uploaded with
        // transpose=false and MSL float4x4 share the same memory layout.
        let mut b = Vec::with_capacity(64);
        for v in values {
            b.extend_from_slice(&v.to_ne_bytes());
        }
        self.stage_uniform(program, name, b);
    }

    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        self.encode_draw(|enc| unsafe {
            send_draw_primitives(
                enc,
                sel("drawPrimitives:vertexStart:vertexCount:"),
                MTL_PRIMITIVE_TYPE_TRIANGLE,
                first.max(0) as NsUInteger,
                count.max(0) as NsUInteger,
            );
        });
    }

    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        let index_buffer = self
            .vaos
            .get(&self.current_vao)
            .map_or(0, |v| v.index_buffer);
        let Some(&ibuf) = self.buffers.get(&index_buffer) else {
            return;
        };
        self.encode_draw(|enc| unsafe {
            send_draw_indexed(
                enc,
                sel("drawIndexedPrimitives:indexCount:indexType:indexBuffer:indexBufferOffset:"),
                MTL_PRIMITIVE_TYPE_TRIANGLE,
                count.max(0) as NsUInteger,
                MTL_INDEX_TYPE_UINT32,
                ibuf,
                byte_offset.max(0) as NsUInteger,
            );
        });
    }

    /// `gfx.clear` — color **and** depth (SPEC § 7.4), unlike
    /// `Window.clear` above.
    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        let _pool = AutoreleasePool::push();
        unsafe {
            self.resize_if_needed();
            let cmd = send0(self.queue, sel("commandBuffer"));
            if cmd.is_null() {
                return;
            }
            let rpd = self.clear_pass_descriptor(r, g, b, a, true);
            if rpd.is_null() {
                return;
            }
            let enc = send1_obj_obj(cmd, sel("renderCommandEncoderWithDescriptor:"), rpd);
            if !enc.is_null() {
                send0_void(enc, sel("endEncoding"));
            }
            send0_void(cmd, sel("commit"));
        }
    }

    pub fn set_depth_test(&mut self, enabled: bool) {
        self.depth_test = enabled;
    }

    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.viewport_rect = Some((x, y, w, h));
    }

    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        let (w, h) = (w.max(0), h.max(0));
        let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
        if w == 0 || h == 0 {
            return out;
        }
        let _pool = AutoreleasePool::push();
        unsafe {
            // Drain the queue: an empty command buffer completing means every
            // previously committed draw has too (the queue serializes).
            let cmd = send0(self.queue, sel("commandBuffer"));
            if !cmd.is_null() {
                send0_void(cmd, sel("commit"));
                send0_void(cmd, sel("waitUntilCompleted"));
            }

            // GL coords (bottom-left origin) → Metal coords (top-left):
            // my = target_h - y - h. Read only the intersection with the
            // target; anything outside stays zeroed (GL leaves out-of-bounds
            // reads undefined — all this backend's golden pins are inside).
            let gx0 = x.max(0);
            let gy0 = y.max(0);
            let gx1 = (x + w).min(self.target_w);
            let gy1 = (y + h).min(self.target_h);
            if gx0 >= gx1 || gy0 >= gy1 {
                return out;
            }
            let iw = (gx1 - gx0) as usize;
            let ih = (gy1 - gy0) as usize;
            let my = self.target_h - gy1; // flipped top edge of the region
            let stride = iw * 4;
            let mut tmp = vec![0u8; stride * ih];
            send_get_bytes(
                self.target,
                sel("getBytes:bytesPerRow:fromRegion:mipmapLevel:"),
                tmp.as_mut_ptr() as *mut c_void,
                stride as NsUInteger,
                MtlRegion {
                    x: gx0 as NsUInteger,
                    y: my as NsUInteger,
                    z: 0,
                    w: iw as NsUInteger,
                    h: ih as NsUInteger,
                    d: 1,
                },
                0,
            );
            // Metal hands back top-down BGRA; GL's contract is bottom-up
            // RGBA. Reverse rows and swizzle while scattering into the
            // requested (possibly larger) output rect.
            let out_w = w as usize;
            for row in 0..ih {
                // tmp row `row` is Metal-top-down; its GL y is gy1-1-row.
                let gl_y = (gy1 - 1) as usize - row;
                let out_row = gl_y - y.max(0) as usize + (gy0 - y.max(0)) as usize;
                let src = &tmp[row * stride..row * stride + stride];
                let dst_base = out_row * out_w * 4 + ((gx0 - x.max(0)) as usize) * 4;
                for px in 0..iw {
                    let b = src[px * 4];
                    let g = src[px * 4 + 1];
                    let r = src[px * 4 + 2];
                    let a = src[px * 4 + 3];
                    let d = dst_base + px * 4;
                    out[d] = r;
                    out[d + 1] = g;
                    out[d + 2] = b;
                    out[d + 3] = a;
                }
            }
        }
        out
    }

    // -----------------------------------------------------------------
    // Internals.
    // -----------------------------------------------------------------

    /// Build the clear-pass descriptor shared by `clear`/`gfx_clear`
    /// (autoreleased — callers hold a pool). Clear passes never attach depth
    /// unless clearing it: a pass with no draws needs no depth attachment,
    /// and `Window.clear`'s contract is color-only.
    unsafe fn clear_pass_descriptor(
        &self,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
        clear_depth: bool,
    ) -> *mut Object {
        let rpd = send0(class("MTLRenderPassDescriptor"), sel("renderPassDescriptor"));
        if rpd.is_null() {
            return rpd;
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
        if clear_depth {
            let datt = send0(rpd, sel("depthAttachment"));
            send1_obj(datt, sel("setTexture:"), self.depth);
            send1_uint_void(datt, sel("setLoadAction:"), MTL_LOAD_ACTION_CLEAR);
            send1_f64_void(datt, sel("setClearDepth:"), 1.0);
            send1_uint_void(datt, sel("setStoreAction:"), MTL_STORE_ACTION_STORE);
        }
        rpd
    }

    /// The 2-state depth-stencil cache, built on first use (see the field
    /// doc comment). A nil result (never observed in practice) just leaves
    /// the encoder's default state, which equals "depth test off".
    unsafe fn ensure_depth_states(&mut self) {
        if !self.depth_state_off.is_null() {
            return;
        }
        for (enabled, slot) in [(false, 0usize), (true, 1usize)] {
            let desc_alloc = send0(class("MTLDepthStencilDescriptor"), sel("alloc"));
            if desc_alloc.is_null() {
                return;
            }
            let desc = send0(desc_alloc, sel("init"));
            if desc.is_null() {
                return;
            }
            send1_uint_void(
                desc,
                sel("setDepthCompareFunction:"),
                if enabled {
                    MTL_COMPARE_FUNCTION_LESS
                } else {
                    MTL_COMPARE_FUNCTION_ALWAYS
                },
            );
            send1_bool_void(
                desc,
                sel("setDepthWriteEnabled:"),
                if enabled { OBJC_YES } else { OBJC_NO },
            );
            let state = send1_obj_obj(self.device, sel("newDepthStencilStateWithDescriptor:"), desc);
            send0_void(desc, sel("release"));
            if slot == 0 {
                self.depth_state_off = state;
            } else {
                self.depth_state_on = state;
            }
        }
    }

    /// Get-or-build the `MTLRenderPipelineState` for (current program,
    /// vertex layout), filling the program's reflected uniform layouts on
    /// first build. Failures are reported once per layout then skipped —
    /// the GL analog (drawing with mismatched program/attrib state) is
    /// silent undefined rendering, so a loud-once eprintln is strictly more
    /// diagnosable.
    unsafe fn ensure_pso(
        &mut self,
        program: u32,
        attribs: &[(u32, Attrib)],
        fp: u64,
    ) -> Option<*mut Object> {
        let rec = self.programs.get_mut(&program)?;
        if let Some(&pso) = rec.psos.get(&fp) {
            return Some(pso);
        }
        if rec.failed_layouts.contains(&fp) {
            return None;
        }

        let desc_alloc = send0(class("MTLRenderPipelineDescriptor"), sel("alloc"));
        let desc = if desc_alloc.is_null() {
            std::ptr::null_mut()
        } else {
            send0(desc_alloc, sel("init"))
        };
        if desc.is_null() {
            rec.failed_layouts.insert(fp);
            return None;
        }
        send1_obj(desc, sel("setVertexFunction:"), rec.vfun);
        send1_obj(desc, sel("setFragmentFunction:"), rec.ffun);

        // The VAO shim's replay target: attribute i reads from the buffer
        // bound at index 1+i (0 is the uniform struct), offset 0 within each
        // stride element — the base offset lives in the buffer binding.
        let vdesc = send0(class("MTLVertexDescriptor"), sel("vertexDescriptor"));
        let vattrs = send0(vdesc, sel("attributes"));
        let vlayouts = send0(vdesc, sel("layouts"));
        for (i, a) in attribs {
            let size = a.size.clamp(1, 4) as NsUInteger;
            let att = send1_uint_obj(vattrs, sel("objectAtIndexedSubscript:"), *i as NsUInteger);
            send1_uint_void(att, sel("setFormat:"), MTL_VERTEX_FORMAT_FLOAT + (size - 1));
            send1_uint_void(att, sel("setOffset:"), 0);
            send1_uint_void(att, sel("setBufferIndex:"), (*i + 1) as NsUInteger);
            let lay = send1_uint_obj(
                vlayouts,
                sel("objectAtIndexedSubscript:"),
                (*i + 1) as NsUInteger,
            );
            send1_uint_void(lay, sel("setStride:"), effective_stride(a) as NsUInteger);
        }
        send1_obj(desc, sel("setVertexDescriptor:"), vdesc);

        let catts = send0(desc, sel("colorAttachments"));
        let catt = send1_uint_obj(catts, sel("objectAtIndexedSubscript:"), 0);
        send1_uint_void(catt, sel("setPixelFormat:"), MTL_PIXEL_FORMAT_BGRA8_UNORM);
        send1_uint_void(
            desc,
            sel("setDepthAttachmentPixelFormat:"),
            MTL_PIXEL_FORMAT_DEPTH32_FLOAT,
        );

        let mut refl: *mut Object = std::ptr::null_mut();
        let mut err: *mut Object = std::ptr::null_mut();
        let pso = send_new_pipeline(
            self.device,
            sel("newRenderPipelineStateWithDescriptor:options:reflection:error:"),
            desc,
            MTL_PIPELINE_OPTION_REFLECTION,
            &mut refl,
            &mut err,
        );
        send0_void(desc, sel("release"));
        if pso.is_null() {
            let msg = if err.is_null() {
                "unknown pipeline-state error".to_string()
            } else {
                nsstring_to_owned(send0(err, sel("localizedDescription")))
            };
            eprintln!(
                "gfx (metal): pipeline build failed for program {program} \
                 (shader/vertex-layout mismatch?): {msg}"
            );
            rec.failed_layouts.insert(fp);
            return None;
        }
        if rec.vertex_layout.is_none() && !refl.is_null() {
            rec.vertex_layout = Some(parse_stage_layout(send0(refl, sel("vertexArguments"))));
            rec.fragment_layout = Some(parse_stage_layout(send0(refl, sel("fragmentArguments"))));
        }
        rec.psos.insert(fp, pso);
        Some(pso)
    }

    /// The shared draw-encoding path: resolve every piece of GL-shaped state
    /// (pipeline, depth state, viewport, vertex bindings, uniforms,
    /// textures) into one fresh loadAction=Load render pass, then let the
    /// caller issue the actual draw. A missing precondition (no program, no
    /// pipeline) skips the draw — the GL analog is silent undefined
    /// rendering, so skipping is never *less* correct.
    fn encode_draw(&mut self, draw: impl FnOnce(*mut Object)) {
        if self.current_program == 0 {
            return;
        }
        let _pool = AutoreleasePool::push();
        unsafe {
            self.resize_if_needed();
            self.ensure_depth_states();

            // Snapshot the current VAO's layout (fingerprint + replay list).
            let attribs: Vec<(u32, Attrib)> = self
                .vaos
                .get(&self.current_vao)
                .map(|v| v.attribs.iter().map(|(i, a)| (*i, *a)).collect())
                .unwrap_or_default();
            let fp = layout_fingerprint(&attribs);
            let program = self.current_program;
            let Some(pso) = self.ensure_pso(program, &attribs, fp) else {
                return;
            };

            // Stage the uniform structs for both stages from the reflected
            // layouts (names the shader doesn't declare are ignored, like
            // glGetUniformLocation returning -1).
            let rec = &self.programs[&program];
            let stage_bytes = |layout: &Option<StageLayout>| -> Vec<u8> {
                let Some(l) = layout else { return Vec::new() };
                let mut buf = vec![0u8; l.size];
                for (name, bytes) in &rec.uniforms {
                    if let Some(&off) = l.offsets.get(name) {
                        let end = (off + bytes.len()).min(buf.len());
                        if off < end {
                            buf[off..end].copy_from_slice(&bytes[..end - off]);
                        }
                    }
                }
                buf
            };
            let vbytes = stage_bytes(&rec.vertex_layout);
            let fbytes = stage_bytes(&rec.fragment_layout);

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
            send1_uint_void(att, sel("setLoadAction:"), MTL_LOAD_ACTION_LOAD);
            send1_uint_void(att, sel("setStoreAction:"), MTL_STORE_ACTION_STORE);
            let datt = send0(rpd, sel("depthAttachment"));
            send1_obj(datt, sel("setTexture:"), self.depth);
            send1_uint_void(datt, sel("setLoadAction:"), MTL_LOAD_ACTION_LOAD);
            send1_uint_void(datt, sel("setStoreAction:"), MTL_STORE_ACTION_STORE);

            let enc = send1_obj_obj(cmd, sel("renderCommandEncoderWithDescriptor:"), rpd);
            if enc.is_null() {
                send0_void(cmd, sel("commit"));
                return;
            }
            send1_obj(enc, sel("setRenderPipelineState:"), pso);

            let dstate = if self.depth_test {
                self.depth_state_on
            } else {
                self.depth_state_off
            };
            if !dstate.is_null() {
                send1_obj(enc, sel("setDepthStencilState:"), dstate);
            }

            // Viewport: GL bottom-left rect → Metal top-left, clamped to the
            // target (Metal validation requires it; GL just scissorlessly
            // clips). None = full target, GL's initial state.
            let (vx, vy, vw, vh) = self
                .viewport_rect
                .unwrap_or((0, 0, self.target_w, self.target_h));
            let vx0 = vx.clamp(0, self.target_w);
            let vy0 = vy.clamp(0, self.target_h);
            let vx1 = (vx + vw).clamp(0, self.target_w);
            let vy1 = (vy + vh).clamp(0, self.target_h);
            if vx1 > vx0 && vy1 > vy0 {
                send1_viewport_void(
                    enc,
                    sel("setViewport:"),
                    MtlViewport {
                        origin_x: vx0 as f64,
                        origin_y: (self.target_h - vy1) as f64,
                        width: (vx1 - vx0) as f64,
                        height: (vy1 - vy0) as f64,
                        znear: 0.0,
                        zfar: 1.0,
                    },
                );
            }

            // VAO replay: bind each attribute's backing buffer at 1+i with
            // the recorded base offset (resolved to a live MTLBuffer *now*,
            // so re-uploads to the same handle are honored).
            for (i, a) in &attribs {
                if let Some(&buf) = self.buffers.get(&a.buffer) {
                    send_set_vertex_buffer(
                        enc,
                        sel("setVertexBuffer:offset:atIndex:"),
                        buf,
                        a.offset.max(0) as NsUInteger,
                        (*i + 1) as NsUInteger,
                    );
                }
            }

            // Uniform structs (inline constants — every gfx uniform is far
            // below Metal's 4 KiB setBytes limit).
            if !vbytes.is_empty() {
                send_set_bytes(
                    enc,
                    sel("setVertexBytes:length:atIndex:"),
                    vbytes.as_ptr() as *const c_void,
                    vbytes.len() as NsUInteger,
                    0,
                );
            }
            if !fbytes.is_empty() {
                send_set_bytes(
                    enc,
                    sel("setFragmentBytes:length:atIndex:"),
                    fbytes.as_ptr() as *const c_void,
                    fbytes.len() as NsUInteger,
                    0,
                );
            }

            // Texture units → [[texture(unit)]].
            for (unit, tex_id) in &self.unit_bindings {
                if let Some(&tex) = self.textures.get(tex_id) {
                    send2_obj_uint_void(
                        enc,
                        sel("setFragmentTexture:atIndex:"),
                        tex,
                        *unit as NsUInteger,
                    );
                }
            }

            draw(enc);
            send0_void(enc, sel("endEncoding"));
            send0_void(cmd, sel("commit"));
        }
    }

    /// Recreate the offscreen color+depth targets and the layer's
    /// `drawableSize` when the window has been live-resized (`poll` keeps
    /// `cocoa.width/height` current), so the whole-texture blit's
    /// size/format precondition keeps holding. The first frame after a
    /// resize starts from fresh (blank) targets — the same single-frame
    /// artifact GL's resize has.
    unsafe fn resize_if_needed(&mut self) {
        let w = self.cocoa.width.max(1);
        let h = self.cocoa.height.max(1);
        if w == self.target_w && h == self.target_h {
            return;
        }
        let Ok(t) = new_offscreen_texture(
            self.device,
            MTL_PIXEL_FORMAT_BGRA8_UNORM,
            MTL_STORAGE_MODE_SHARED,
            MTL_TEXTURE_USAGE_RENDER_TARGET | MTL_TEXTURE_USAGE_SHADER_READ,
            w,
            h,
        ) else {
            return;
        };
        let Ok(d) = new_offscreen_texture(
            self.device,
            MTL_PIXEL_FORMAT_DEPTH32_FLOAT,
            MTL_STORAGE_MODE_PRIVATE,
            MTL_TEXTURE_USAGE_RENDER_TARGET,
            w,
            h,
        ) else {
            send0_void(t, sel("release"));
            return;
        };
        send0_void(self.target, sel("release"));
        send0_void(self.depth, sel("release"));
        self.target = t;
        self.depth = d;
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

    pub fn teardown(self) {
        unsafe {
            for (_, buf) in self.buffers {
                send0_void(buf, sel("release"));
            }
            for (_, tex) in self.textures {
                send0_void(tex, sel("release"));
            }
            for (_, rec) in self.programs {
                send0_void(rec.vfun, sel("release"));
                send0_void(rec.ffun, sel("release"));
                for (_, pso) in rec.psos {
                    send0_void(pso, sel("release"));
                }
            }
            if !self.depth_state_off.is_null() {
                send0_void(self.depth_state_off, sel("release"));
            }
            if !self.depth_state_on.is_null() {
                send0_void(self.depth_state_on, sel("release"));
            }
            send0_void(self.depth, sel("release"));
            send0_void(self.target, sel("release"));
            send0_void(self.layer, sel("release"));
            send0_void(self.queue, sel("release"));
            send0_void(self.device, sel("release"));
        }
        self.cocoa.teardown();
    }
}

/// Parse one stage's reflected arguments (an `NSArray<MTLArgument>`, or nil)
/// into the `[[buffer(0)]]` uniform-struct layout: total size plus each
/// member's name → byte offset. A stage with no buffer-0 argument yields an
/// empty layout (that stage simply takes no uniforms).
unsafe fn parse_stage_layout(args: *mut Object) -> StageLayout {
    if args.is_null() {
        return StageLayout::empty();
    }
    let count = send0_uint(args, sel("count"));
    for i in 0..count {
        let arg = send1_uint_obj(args, sel("objectAtIndex:"), i);
        if arg.is_null() {
            continue;
        }
        let ty = send0_uint(arg, sel("type"));
        let index = send0_uint(arg, sel("index"));
        if ty != MTL_ARGUMENT_TYPE_BUFFER || index != 0 {
            continue;
        }
        let size = send0_uint(arg, sel("bufferDataSize")) as usize;
        let mut offsets = HashMap::new();
        let stype = send0(arg, sel("bufferStructType"));
        if !stype.is_null() {
            let members = send0(stype, sel("members"));
            if !members.is_null() {
                let mcount = send0_uint(members, sel("count"));
                for m in 0..mcount {
                    let member = send1_uint_obj(members, sel("objectAtIndex:"), m);
                    if member.is_null() {
                        continue;
                    }
                    let name = nsstring_to_owned(send0(member, sel("name")));
                    let offset = send0_uint(member, sel("offset")) as usize;
                    offsets.insert(name, offset);
                }
            }
        }
        return StageLayout { size, offsets };
    }
    StageLayout::empty()
}
