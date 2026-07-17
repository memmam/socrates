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
//! **Phase 0 status**: [`vulkan::Inner`] is deliberately *uninhabited*
//! (its `create` always `Err`s — see its module docs), so every `Vulkan`
//! match arm below is the statically-unreachable `match *i {}`. The WSI
//! phase replaces those arms with real forwards, exactly as `x11/mod.rs`'s
//! arms became real when `x11/vulkan.rs` grew its implementation.
//!
//! Win32 primitives (window creation, the message pump, key mapping) that
//! both backends need are factored into [`shared`], composed by each
//! backend rather than duplicated.

// With `gl` off (a vulkan-only build), every dispatch method's parameters
// flow only into the statically-unreachable `Vulkan` arm — the same
// Phase-0 shape `x11/mod.rs` had, silenced the same way until the WSI
// phase makes the arms real.
#![cfg_attr(not(feature = "gl"), allow(unused_variables))]

#[cfg(feature = "gl")]
mod gl;
mod shared;
#[cfg(feature = "vulkan")]
mod vulkan;

/// Either a live OpenGL/WGL window ([`gl::Inner`]) or a live Vulkan window
/// ([`vulkan::Inner`], uninhabited until the WSI phase) — see the module
/// doc comment for why this is an enum rather than a plain struct.
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
    /// variant. Phase 0: always a clean `Err` (see `vulkan`'s module docs).
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
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn key_down(&self, name: &str) -> bool {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.key_down(name),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn mouse(&self) -> (f64, f64) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.mouse(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn width(&self) -> i32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.width(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn height(&self) -> i32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.height(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn should_close(&self) -> bool {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.should_close(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn backend_name(&self) -> &'static str {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.backend_name(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.clear(r, g, b, a),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn swap_buffers(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.swap_buffers(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn make_current(&mut self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.make_current(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn compile_program_spirv(&mut self, vs: &[u8], fs: &[u8]) -> Result<u32, String> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.compile_program_spirv(vs, fs),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.compile_program(vertex_src, fragment_src),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn use_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.use_program(program),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn delete_program(&mut self, program: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_program(program),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn create_buffer(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_buffer(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn delete_buffer(&mut self, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_buffer(buffer),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_buffer(kind, buffer),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn upload_buffer(&mut self, kind: crate::window::GfxBufferKind, data: &[u8], dynamic: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_buffer(kind, data, dynamic),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn create_vertex_array(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_vertex_array(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn bind_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_vertex_array(vao),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn delete_vertex_array(&mut self, vao: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_vertex_array(vao),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_vertex_attrib(index, size, stride, offset),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn disable_vertex_attrib(&mut self, index: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.disable_vertex_attrib(index),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn create_texture(&mut self) -> u32 {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.create_texture(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn delete_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.delete_texture(tex),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn bind_texture(&mut self, tex: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.bind_texture(tex),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn active_texture_unit(&mut self, unit: u32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.active_texture_unit(unit),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.upload_texture(data, width, height, has_alpha),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_int(program, name, v),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_float(program, name, v),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec2(program, name, x, y),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec3(program, name, x, y, z),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_vec4(program, name, x, y, z, w),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_uniform_mat4(program, name, values),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_arrays(first, count),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.draw_elements(count, byte_offset),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.gfx_clear(r, g, b, a),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn set_depth_test(&mut self, enabled: bool) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.set_depth_test(enabled),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.viewport(x, y, w, h),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.read_pixels(x, y, w, h),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match *i {},
        }
    }
    pub fn teardown(self) {
        match self {
            #[cfg(feature = "gl")]
            Inner::Gl(i) => i.teardown(),
            #[cfg(feature = "vulkan")]
            Inner::Vulkan(i) => match i {},
        }
    }
}

#[cfg(test)]
mod tests {
    /// `create_vulkan` end-to-end through the enum dispatch: Phase 0's
    /// stub (feature on) and the feature-off stub both return a clean,
    /// prefixed `Err` — deterministic in every build shape, no panic
    /// paths. (The `Ok` arm becomes reachable when the WSI phase lands,
    /// at which point this test grows the x11 version's real-window arm.)
    #[test]
    fn create_vulkan_errs_cleanly_in_phase0() {
        match super::Inner::create_vulkan("fable window test", 320, 240) {
            Ok(_) => unreachable!("Phase 0 create_vulkan cannot succeed"),
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
