//! Windows window backend for the `window` namespace — dispatches between
//! two coexisting rendering backends, OpenGL/WGL ([`gl`]) and Vulkan
//! ([`vulkan`]), never replacing one with the other. The exact structural
//! twin of `x11/mod.rs`'s GL/Vulkan dispatch and `macos/mod.rs`'s GL/Metal
//! dispatch (see the standing native-backend roadmap in `CLAUDE.md`).
//!
//! **Why an enum, not a single struct**: [`super::WindowHandle`]'s one
//! `inner: Option<PlatformInner>` field can only ever hold one concrete
//! Rust type, and a type alias (`PlatformInner`) resolves to exactly one
//! type per compiled binary — only a sum type underneath the alias lets one
//! `WindowHandle` transparently hold either backend's live window in the
//! same compiled program. Every method below is a small `match` forwarding
//! to whichever variant is live.
//!
//! **Status**: [`vulkan::Inner`] is at full parity — window lifecycle
//! (create/clear/present/events/teardown) and the whole `gfx.*` draw-call
//! surface forward to the shared Vulkan windowing core
//! (`window/vulkan.rs`), the exact code the x11 backend's lavapipe pixel
//! asserts prove in CI.
//!
//! Win32 primitives (window creation, the message pump, key mapping) that
//! both backends need are factored into [`shared`], composed by each
//! backend rather than duplicated.


#[cfg(feature = "gl")]
mod gl;
mod shared;
#[cfg(feature = "vulkan")]
mod vulkan;

/// Either a live OpenGL/WGL window ([`gl::Inner`]) or a live Vulkan window
/// ([`vulkan::Inner`]) — see the module doc comment for why this is an
/// enum rather than a plain struct.
#[allow(clippy::large_enum_variant)]
pub enum Inner {
    #[cfg(feature = "gl")]
    Gl(gl::Inner),
    #[cfg(feature = "vulkan")]
    Vulkan(vulkan::Inner),
}

impl Inner {
    /// The default `window.create()` entry point — always OpenGL/WGL.
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

    /// The `window.create_vulkan()` entry point — never produces the `Gl`
    /// variant.
    #[cfg(feature = "vulkan")]
    pub fn create_vulkan(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        vulkan::Inner::create(title, w, h).map(Inner::Vulkan)
    }
    #[cfg(not(feature = "vulkan"))]
    pub fn create_vulkan(_title: &str, _w: i32, _h: i32) -> Result<Inner, String> {
        Err(
            "window.create_vulkan: Vulkan windowing support not compiled in (build with \
             --features vulkan)"
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
    pub fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.backend_name(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.backend_name(),
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
    pub fn make_current(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.make_current(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.make_current(),
        }
    }
    pub fn compile_program_spirv(&mut self, vs: &[u8], fs: &[u8]) -> Result<u32, String> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.compile_program_spirv(vs, fs),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.compile_program_spirv(vs, fs),
        }
    }
    pub fn compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.compile_program(vertex_src, fragment_src),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(_i) => {
                let _ = (vertex_src, fragment_src);
                Err(
                    "gfx.compile_program: the Vulkan backend takes SPIR-V binaries, not GLSL \
                     source — use gfx.compile_program_spirv(vertex, fragment)"
                        .to_string(),
                )
            }
        }
    }
    pub fn use_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.use_program(program),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.use_program(program),
        }
    }
    pub fn delete_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_program(program),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.delete_program(program),
        }
    }
    pub fn create_buffer(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_buffer(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.create_buffer(),
        }
    }
    pub fn delete_buffer(&mut self, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_buffer(buffer),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.delete_buffer(buffer),
        }
    }
    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_buffer(kind, buffer),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.bind_buffer(kind, buffer),
        }
    }
    pub fn upload_buffer(&mut self, kind: crate::window::GfxBufferKind, data: &[u8], dynamic: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_buffer(kind, data, dynamic),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.upload_buffer(kind, data, dynamic),
        }
    }
    pub fn create_vertex_array(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_vertex_array(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.create_vertex_array(),
        }
    }
    pub fn bind_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_vertex_array(vao),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.bind_vertex_array(vao),
        }
    }
    pub fn delete_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_vertex_array(vao),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.delete_vertex_array(vao),
        }
    }
    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_vertex_attrib(index, size, stride, offset),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_vertex_attrib(index, size, stride, offset),
        }
    }
    pub fn disable_vertex_attrib(&mut self, index: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.disable_vertex_attrib(index),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.disable_vertex_attrib(index),
        }
    }
    pub fn create_texture(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_texture(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.create_texture(),
        }
    }
    pub fn delete_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_texture(tex),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.delete_texture(tex),
        }
    }
    pub fn bind_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_texture(tex),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.bind_texture(tex),
        }
    }
    pub fn active_texture_unit(&mut self, unit: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.active_texture_unit(unit),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.active_texture_unit(unit),
        }
    }
    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_texture(data, width, height, has_alpha),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.upload_texture(data, width, height, has_alpha),
        }
    }
    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_int(program, name, v),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_uniform_int(program, name, v),
        }
    }
    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_float(program, name, v),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_uniform_float(program, name, v),
        }
    }
    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec2(program, name, x, y),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_uniform_vec2(program, name, x, y),
        }
    }
    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec3(program, name, x, y, z),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_uniform_vec3(program, name, x, y, z),
        }
    }
    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec4(program, name, x, y, z, w),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_uniform_vec4(program, name, x, y, z, w),
        }
    }
    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_mat4(program, name, values),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_uniform_mat4(program, name, values),
        }
    }
    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_arrays(first, count),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.draw_arrays(first, count),
        }
    }
    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_elements(count, byte_offset),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.draw_elements(count, byte_offset),
        }
    }
    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.gfx_clear(r, g, b, a),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.gfx_clear(r, g, b, a),
        }
    }
    pub fn set_depth_test(&mut self, enabled: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_depth_test(enabled),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.set_depth_test(enabled),
        }
    }
    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.viewport(x, y, w, h),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.viewport(x, y, w, h),
        }
    }
    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.read_pixels(x, y, w, h),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => i.read_pixels(x, y, w, h),
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


#[cfg(test)]
mod tests {
    /// `create_vulkan` end-to-end through the enum dispatch: with the
    /// `vulkan` feature on and a Vulkan loader + device present it opens
    /// a real window (torn down immediately); on a driverless machine or
    /// with the feature off it returns a clean, prefixed `Err`. Every
    /// build shape is deterministic — no panic paths.
    #[test]
    fn create_vulkan_opens_or_errs_cleanly() {
        match super::Inner::create_vulkan("socrates window test", 320, 240) {
            Ok(inner) => {
                assert_eq!(inner.backend_name(), "vulkan");
                inner.teardown();
            }
            Err(err) => {
                assert!(err.contains("window.create_vulkan"), "got: {err}");
                if !cfg!(feature = "vulkan") {
                    assert!(err.contains("not compiled in"), "got: {err}");
                    assert!(err.contains("--features vulkan"), "got: {err}");
                }
            }
        }
    }
}
