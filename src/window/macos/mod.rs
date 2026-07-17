//! macOS (Apple Silicon / `aarch64-apple-darwin` only) window backend for
//! the `window` namespace — dispatches between two coexisting rendering
//! backends, OpenGL/CGL ([`gl`]) and Metal ([`metal`]), never replacing one
//! with the other (see `CLAUDE.md`'s standing Metal exception: it ships
//! additive alongside OpenGL/CGL until and unless Apple itself drops OpenGL
//! on macOS).
//!
//! **Why an enum, not a single struct**: [`super::WindowHandle`]'s one
//! `inner: Option<PlatformInner>` field can only ever hold one concrete Rust
//! type, and a type alias (`PlatformInner`) resolves to exactly one type per
//! compiled binary. The only way one `WindowHandle` can transparently hold
//! *either* a live GL-backed or Metal-backed window within a single compiled
//! program is a sum type underneath the alias — so `Inner` here is a
//! two-variant enum, `#[cfg]`-gated per variant so a build with only one of
//! `gl`/`metal` enabled doesn't try to compile the other's module at all.
//! Every method below is a small `match` that forwards to whichever variant
//! is live; `window/mod.rs`'s generic `WindowHandle` code (shared with
//! `x11/gl.rs`/`win32.rs`, which are plain structs) never needs to know which
//! backend it's talking to.
//!
//! **`window.create()` vs. `window.create_metal()`**: the *default*,
//! unqualified `window.create()` means OpenGL/CGL, unconditionally, exactly
//! as it always has — [`Inner::create`] only ever produces the `Gl` variant.
//! [`Inner::create_metal`] is the new, explicit opt-in that produces the
//! `Metal` variant. Neither can produce the other.
//!
//! Cocoa/AppKit primitives (window creation, the event-pump loop) that
//! both backends need are factored into [`shared`], composed by each
//! backend rather than duplicated.

#[cfg(feature = "gl")]
mod gl;
#[cfg(feature = "metal")]
mod metal;
mod shared;

/// Either a live OpenGL/CGL window ([`gl::Inner`]) or a live Metal window
/// ([`metal::Inner`]) — see the module doc comment for why this is an enum
/// rather than a plain struct.
///
/// `gl::Inner` (mainly its 45-function-pointer `GlFns` table) is much larger
/// than `metal::Inner`, which clippy flags — deliberately not boxed: this
/// namespace is explicitly single-window scoped (see `src/window/mod.rs`'s
/// module doc comment), so at most one `Inner` is ever live at a time, and
/// every `gfx.*` call already goes through this enum's `match` on the hot
/// path — an extra `Box` indirection there would cost more in practice than
/// the one-time size difference of a single value ever does.
#[allow(clippy::large_enum_variant)]
pub enum Inner {
    #[cfg(feature = "gl")]
    Gl(gl::Inner),
    #[cfg(feature = "metal")]
    Metal(metal::Inner),
}

/// Is this the process's real main thread — the only thread AppKit allows
/// window creation on? Exposed at crate level for `lib.rs`'s
/// `run_capture_path` (the `fable test` runner), which runs test bodies
/// inline when — and only when — windowing could actually work on this
/// thread; see its doc comment for the full reasoning.
pub fn is_main_thread() -> bool {
    shared::is_main_thread()
}

impl Inner {
    /// The default `window.create()` entry point — always OpenGL/CGL.
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

    /// The `window.create_metal()` entry point — see the module doc
    /// comment; never produces the `Gl` variant.
    #[cfg(feature = "metal")]
    pub fn create_metal(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        metal::Inner::create(title, w, h).map(Inner::Metal)
    }
    #[cfg(not(feature = "metal"))]
    pub fn create_metal(_title: &str, _w: i32, _h: i32) -> Result<Inner, String> {
        Err(
            "window.create_metal: Metal windowing support not compiled in (build with \
             --features metal, aarch64-apple-darwin only)"
                .to_string(),
        )
    }

    pub fn poll(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.poll(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.poll(),
        }
    }

    pub fn key_down(&self, name: &str) -> bool {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.key_down(name),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.key_down(name),
        }
    }

    pub fn mouse(&self) -> (f64, f64) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.mouse(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.mouse(),
        }
    }

    pub fn width(&self) -> i32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.width(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.width(),
        }
    }

    pub fn height(&self) -> i32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.height(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.height(),
        }
    }

    pub fn should_close(&self) -> bool {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.should_close(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.should_close(),
        }
    }

    /// `"opengl"` or `"metal"` — see `WindowHandle::backend_name`'s doc
    /// comment (`src/window/mod.rs`) for why this exists.
    pub fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(_) => "opengl",
            #[cfg(feature = "metal")]
            Inner::Metal(_) => "metal",
        }
    }

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.clear(r, g, b, a),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.clear(r, g, b, a),
        }
    }

    pub fn swap_buffers(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.swap_buffers(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.swap_buffers(),
        }
    }

    // -----------------------------------------------------------------
    // gfx.* — forwarded to whichever backend is live. Both variants
    // implement the full draw-call surface (GL since v0.8; Metal since
    // Phase 2 of the Metal arc) with identical observable semantics; the
    // shader *source text* each expects (GLSL vs. MSL) is the one
    // deliberate per-backend difference — see `metal.rs`'s module docs for
    // the MSL conventions and `WindowHandle::backend_name` for the escape
    // hatch programs branch on.
    // -----------------------------------------------------------------

    pub fn make_current(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.make_current(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.make_current(),
        }
    }

    pub fn compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.compile_program(vertex_src, fragment_src),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.compile_program(vertex_src, fragment_src),
        }
    }

    pub fn use_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.use_program(program),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.use_program(program),
        }
    }

    pub fn delete_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_program(program),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.delete_program(program),
        }
    }

    pub fn create_buffer(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_buffer(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.create_buffer(),
        }
    }

    pub fn delete_buffer(&mut self, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_buffer(buffer),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.delete_buffer(buffer),
        }
    }

    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_buffer(kind, buffer),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.bind_buffer(kind, buffer),
        }
    }

    pub fn upload_buffer(&mut self, kind: crate::window::GfxBufferKind, data: &[u8], dynamic: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_buffer(kind, data, dynamic),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.upload_buffer(kind, data, dynamic),
        }
    }

    pub fn create_vertex_array(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_vertex_array(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.create_vertex_array(),
        }
    }

    pub fn bind_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_vertex_array(vao),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.bind_vertex_array(vao),
        }
    }

    pub fn delete_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_vertex_array(vao),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.delete_vertex_array(vao),
        }
    }

    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_vertex_attrib(index, size, stride, offset),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_vertex_attrib(index, size, stride, offset),
        }
    }

    pub fn disable_vertex_attrib(&mut self, index: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.disable_vertex_attrib(index),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.disable_vertex_attrib(index),
        }
    }

    pub fn create_texture(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_texture(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.create_texture(),
        }
    }

    pub fn delete_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_texture(tex),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.delete_texture(tex),
        }
    }

    pub fn bind_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_texture(tex),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.bind_texture(tex),
        }
    }

    pub fn active_texture_unit(&mut self, unit: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.active_texture_unit(unit),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.active_texture_unit(unit),
        }
    }

    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_texture(data, width, height, has_alpha),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.upload_texture(data, width, height, has_alpha),
        }
    }

    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_int(program, name, v),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_uniform_int(program, name, v),
        }
    }

    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_float(program, name, v),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_uniform_float(program, name, v),
        }
    }

    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec2(program, name, x, y),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_uniform_vec2(program, name, x, y),
        }
    }

    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec3(program, name, x, y, z),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_uniform_vec3(program, name, x, y, z),
        }
    }

    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec4(program, name, x, y, z, w),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_uniform_vec4(program, name, x, y, z, w),
        }
    }

    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_mat4(program, name, values),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_uniform_mat4(program, name, values),
        }
    }

    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_arrays(first, count),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.draw_arrays(first, count),
        }
    }

    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_elements(count, byte_offset),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.draw_elements(count, byte_offset),
        }
    }

    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.gfx_clear(r, g, b, a),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.gfx_clear(r, g, b, a),
        }
    }

    pub fn set_depth_test(&mut self, enabled: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_depth_test(enabled),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.set_depth_test(enabled),
        }
    }

    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.viewport(x, y, w, h),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.viewport(x, y, w, h),
        }
    }

    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.read_pixels(x, y, w, h),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.read_pixels(x, y, w, h),
        }
    }

    pub fn teardown(self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.teardown(),
            #[cfg(feature = "metal")]
            Inner::Metal(i) => i.teardown(),
        }
    }
}
