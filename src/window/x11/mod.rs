//! Linux/X11 window backend for the `window` namespace — dispatches between
//! two coexisting rendering backends, OpenGL/GLX ([`gl`]) and Vulkan
//! ([`vulkan`]), never replacing one with the other. The exact structural
//! twin of `macos/mod.rs`'s GL/Metal dispatch (see the standing native-
//! backend roadmap in `CLAUDE.md`: one backend-neutral Fable-facing
//! surface, thin per-API backends over raw FFI).
//!
//! **Why an enum, not a single struct**: [`super::WindowHandle`]'s one
//! `inner: Option<PlatformInner>` field can only ever hold one concrete Rust
//! type, and a type alias (`PlatformInner`) resolves to exactly one type per
//! compiled binary. The only way one `WindowHandle` can transparently hold
//! *either* a live GL-backed or Vulkan-backed window within a single
//! compiled program is a sum type underneath the alias — so `Inner` here is
//! a two-variant enum, `#[cfg]`-gated per variant so a build with only one
//! of `gl`/`vulkan` enabled doesn't try to compile the other's module at
//! all. Every method below is a small `match` that forwards to whichever
//! variant is live; `window/mod.rs`'s generic `WindowHandle` code (shared
//! with `win32.rs` and `macos/`) never needs to know which backend it's
//! talking to.
//!
//! **`window.create()` vs. `window.create_vulkan()`**: the *default*,
//! unqualified `window.create()` means OpenGL/GLX, unconditionally, exactly
//! as it always has — [`Inner::create`] only ever produces the `Gl` variant.
//! [`Inner::create_vulkan`] is the new, explicit opt-in that produces the
//! `Vulkan` variant. Neither can produce the other.
//!
//! X11/Xlib primitives (window creation, the event-pump loop, the async
//! protocol-error watch) that both backends need are factored into
//! [`shared`], composed by each backend rather than duplicated.

// Temporary until the Vulkan draw-call surface lands: with `gl` off, every
// `gfx.*`-forwarding method's `Gl` match arm (its only real,
// parameter-using arm — the `Vulkan` arm panics via `vulkan_gfx_todo`
// until draw-call parity ships) doesn't exist, so a `--features
// vulkan`-only build sees every one of those parameters as unused.
// `gl.rs`'s callers already exercise all of them whenever `gl` is on, so
// scoping the allowance to "only when `gl` is off" keeps real
// unused-variable detection active for every build that actually
// exercises these methods.
#![cfg_attr(not(feature = "gl"), allow(unused_variables))]

#[cfg(feature = "gl")]
mod gl;
mod shared;
#[cfg(feature = "vulkan")]
mod vulkan;

/// Either a live OpenGL/GLX window ([`gl::Inner`]) or a live Vulkan window
/// ([`vulkan::Inner`]) — see the module doc comment for why this is an enum
/// rather than a plain struct.
///
/// `gl::Inner` (mainly its 45-function-pointer `GlFns` table) is much larger
/// than `vulkan::Inner`, which clippy flags — deliberately not boxed, for
/// the same reason `macos/mod.rs` doesn't box its `Gl` variant: this
/// namespace is explicitly single-window scoped (see `src/window/mod.rs`'s
/// module doc comment), so at most one `Inner` is ever live at a time, and
/// every `gfx.*` call already goes through this enum's `match` on the hot
/// path — an extra `Box` indirection there would cost more in practice than
/// the one-time size difference of a single value ever does.
#[allow(clippy::large_enum_variant)]
pub enum Inner {
    #[cfg(feature = "gl")]
    Gl(gl::Inner),
    #[cfg(feature = "vulkan")]
    Vulkan(vulkan::Inner),
}

impl Inner {
    /// The default `window.create()` entry point — always OpenGL/GLX.
    #[cfg(feature = "gl")]
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        gl::Inner::create(title, w, h).map(Inner::Gl)
    }
    #[cfg(not(feature = "gl"))]
    pub fn create(_title: &str, _w: i32, _h: i32) -> Result<Inner, String> {
        Err(
            "window.create: OpenGL windowing support not compiled in (build with --features gl)"
                .to_string(),
        )
    }

    /// The `window.create_vulkan()` entry point — see the module doc
    /// comment; never produces the `Gl` variant.
    #[cfg(feature = "vulkan")]
    pub fn create_vulkan(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        vulkan::Inner::create(title, w, h).map(Inner::Vulkan)
    }
    #[cfg(not(feature = "vulkan"))]
    pub fn create_vulkan(_title: &str, _w: i32, _h: i32) -> Result<Inner, String> {
        Err(
            "window.create_vulkan: Vulkan windowing support not compiled in (build with \
             --features vulkan, Linux/X11 only for now)"
                .to_string(),
        )
    }

    pub fn poll(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.poll(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.poll(),
        }
    }

    pub fn key_down(&self, name: &str) -> bool {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.key_down(name),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.key_down(name),
        }
    }

    pub fn mouse(&self) -> (f64, f64) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.mouse(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.mouse(),
        }
    }

    pub fn width(&self) -> i32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.width(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.width(),
        }
    }

    pub fn height(&self) -> i32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.height(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.height(),
        }
    }

    pub fn should_close(&self) -> bool {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.should_close(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.should_close(),
        }
    }

    /// `"opengl"` or `"vulkan"` — see `WindowHandle::backend_name`'s doc
    /// comment (`src/window/mod.rs`) for why this exists.
    pub fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(_) => "opengl",
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_) => "vulkan",
        }
    }

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.clear(r, g, b, a),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.clear(r, g, b, a),
        }
    }

    pub fn swap_buffers(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.swap_buffers(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.swap_buffers(),
        }
    }

    // -----------------------------------------------------------------
    // gfx.* — forwarded to whichever backend is live. The `Gl` arms are
    // the full draw-call surface this backend has shipped since v0.8. On
    // the `Vulkan` variant, everything below `make_current` panics via
    // `vulkan_gfx_todo` until the Vulkan draw-call surface lands (SPIR-V
    // pipelines, buffer/texture handle tables, the VAO shim, reflection
    // uniforms) — a live Vulkan window CAN be made current as of Phase 1,
    // so these arms are genuinely reachable and must say clearly what's
    // missing instead of claiming unreachability. The one exception is
    // `compile_program`, whose `Result` return lets it report the same
    // thing as a clean, catchable `Err` — exactly the Metal arc's
    // between-phases precedent (`macos/mod.rs` at Metal Phase 1).
    // -----------------------------------------------------------------

    pub fn make_current(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.make_current(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.make_current(),
        }
    }

    pub fn compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.compile_program(vertex_src, fragment_src),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => Err(
                "gfx.compile_program: not yet implemented on the Vulkan backend (draw-call \
                 parity lands in a later phase; use window.create() / OpenGL for gfx.* today)"
                    .to_string(),
            ),
        }
    }

    pub fn use_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.use_program(program),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn delete_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_program(program),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn create_buffer(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_buffer(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn delete_buffer(&mut self, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_buffer(buffer),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_buffer(kind, buffer),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn upload_buffer(&mut self, kind: crate::window::GfxBufferKind, data: &[u8], dynamic: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_buffer(kind, data, dynamic),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn create_vertex_array(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_vertex_array(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn bind_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_vertex_array(vao),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn delete_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_vertex_array(vao),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_vertex_attrib(index, size, stride, offset),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn disable_vertex_attrib(&mut self, index: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.disable_vertex_attrib(index),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn create_texture(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_texture(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn delete_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_texture(tex),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn bind_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_texture(tex),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn active_texture_unit(&mut self, unit: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.active_texture_unit(unit),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_texture(data, width, height, has_alpha),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_int(program, name, v),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_float(program, name, v),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec2(program, name, x, y),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec3(program, name, x, y, z),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec4(program, name, x, y, z, w),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_mat4(program, name, values),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_arrays(first, count),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_elements(count, byte_offset),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.gfx_clear(r, g, b, a),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn set_depth_test(&mut self, enabled: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_depth_test(enabled),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.viewport(x, y, w, h),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.read_pixels(x, y, w, h),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => vulkan_gfx_todo(),
        }
    }

    pub fn teardown(self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.teardown(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.teardown(),
        }
    }
}

/// The reachable-but-unimplemented `gfx.*` arms' shared panic — a live
/// Vulkan window exists as of Phase 1, so unlike the Phase-0 stub these
/// paths CAN be hit and must say what's missing (removed when draw-call
/// parity lands, the way `macos/mod.rs`'s `metal_gfx_todo` was).
#[cfg(feature = "vulkan")]
fn vulkan_gfx_todo() -> ! {
    panic!(
        "gfx: draw calls are not yet implemented on the Vulkan backend (draw-call parity \
         lands in a later phase; use window.create() / OpenGL for gfx.* today)"
    )
}

#[cfg(test)]
mod tests {
    /// `create_vulkan` end-to-end through the enum dispatch: with the
    /// `vulkan` feature on and a display + Vulkan device present it opens
    /// a real window (torn down immediately); in a headless environment
    /// or with the feature off it returns a clean, prefixed `Err`. Every
    /// build shape is deterministic — no panic paths.
    #[test]
    fn create_vulkan_opens_or_errs_cleanly() {
        match super::Inner::create_vulkan("fable window test", 320, 240) {
            Ok(inner) => {
                assert_eq!(inner.backend_name(), "vulkan");
                inner.teardown();
            }
            Err(err) => {
                // Both the feature-off stub and every runtime failure mode
                // (no display, no loader, no device) carry the entry-point
                // prefix, so the message is actionable either way.
                assert!(err.contains("window.create_vulkan"), "got: {err}");
                if !cfg!(feature = "vulkan") {
                    assert!(err.contains("not compiled in"), "got: {err}");
                    assert!(err.contains("--features vulkan"), "got: {err}");
                }
            }
        }
    }
}
