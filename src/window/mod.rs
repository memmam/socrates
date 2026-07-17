//! Windowing for the `window` builtin namespace — the GLFW-equivalent piece
//! of the native-OpenGL roadmap (`std.glm`, v0.8, already shipped the math
//! side; the Linux/X11/GLX backend shipped after that). Scope: window
//! creation, event polling, keyboard/mouse state, and a trivial
//! clear-color-and-swap-buffers, enough to prove the whole pipe
//! end-to-end. The backend-neutral GL draw-call layer (shaders, programs,
//! buffers, VAOs, textures, uniforms, draw calls) built on top of this
//! module's per-platform `GlFns` table is the separate `gfx` namespace
//! (v0.8): `natives.rs`'s `gfx.*` match arms call into the `gl_*` methods
//! on [`WindowHandle`] below, which forward to each platform backend's own
//! same-named methods.
//!
//! This module is **always compiled**; only its internals are gated on the
//! `gl`/`metal`/`vulkan` cargo features, mirroring `src/gpu.rs` exactly.
//! All three features add **zero** Cargo dependencies — it is raw FFI to
//! system libraries and frameworks (X11/Win32 linked normally,
//! GL/GLX/WGL/Vulkan resolved with `dlopen`+`dlsym` or
//! `LoadLibrary`+`GetProcAddress` at runtime, Cocoa/NSOpenGL/Metal messaged
//! via `objc_msgSend`) — so `cargo tree -e normal` stays a single line with
//! or without any of them.
//!
//! Three platform backends, each behind its own submodule: [`x11`] (Linux,
//! any arch), [`win32`] (Windows, any arch, `gl` only), [`macos`]
//! (macOS — gated on `target_arch = "aarch64"` **in addition to**
//! `target_os`, not just by convention: it sidesteps the struct-return
//! `objc_msgSend_stret` ABI split that only exists on x86_64, so an
//! accidental `x86_64-apple-darwin` build must not silently compile the same
//! code in and hit that split at runtime). `win32` exposes exactly one
//! `Inner` struct, aliased directly to `PlatformInner`; `macos` and `x11`
//! each carry **two** independently-togglable backends (OpenGL plus,
//! respectively, `metal` — additive per CLAUDE.md's standing Metal
//! exception, never a replacement — and `vulkan`), so their `Inner` (still
//! aliased to `PlatformInner`) is a small enum over both, letting a single
//! compiled binary open either kind of window at runtime via [`create`],
//! [`create_metal`], or [`create_vulkan`] (see `macos/mod.rs`'s module docs
//! for why an enum). Either way, [`WindowHandle`]'s method bodies are
//! written once and never see which platform (or which backend) they're
//! actually running on, except for the one deliberate escape hatch,
//! [`WindowHandle::backend_name`] (shader *source text* is inherently
//! GLSL vs. MSL vs. SPIR-V). On any other target (or with the relevant
//! feature off), [`create`]/[`create_metal`]/[`create_vulkan`] degrade
//! gracefully to `Err`, the same way `gpu::run` does without the `gpu`
//! feature.
//!
//! # Deliberate deviation from `Worker`
//!
//! `Worker` (`src/worker.rs`) has no `Drop` impl: a GC'd-away worker handle
//! just detaches its OS thread — cheap, plentiful, fine to leak until
//! process exit. A GL context + window is a comparatively scarce OS/GPU
//! resource, so [`WindowHandle`] tears down eagerly: both the explicit
//! `close()` method and its `Drop` impl run the same idempotent teardown,
//! so a program that opens/closes many windows in a loop actually reclaims
//! them as they're collected, not only at process exit. Idempotency is via
//! an `Option`-wrapped inner state — `close()`/`Drop` both `take()` it, so a
//! second teardown is a no-op (the same shape as `WorkerHandle::join`'s
//! cached-result idempotency).

#[cfg(all(any(feature = "gl", feature = "vulkan"), target_os = "linux"))]
pub mod x11;
#[cfg(all(any(feature = "gl", feature = "vulkan"), target_os = "linux"))]
use x11::Inner as PlatformInner;

#[cfg(all(any(feature = "gl", feature = "vulkan"), target_os = "windows"))]
pub mod win32;
#[cfg(all(any(feature = "gl", feature = "vulkan"), target_os = "windows"))]
use win32::Inner as PlatformInner;

#[cfg(all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64"))]
pub mod macos;
#[cfg(all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64"))]
use macos::Inner as PlatformInner;

/// Buffer-binding target for `gfx.bind_buffer`/`gfx.upload_buffer` (v0.8):
/// mirrors `GL_ARRAY_BUFFER` / `GL_ELEMENT_ARRAY_BUFFER` as a plain Rust enum
/// so the raw GL enum values stay encapsulated inside each platform backend
/// (`x11/gl.rs`/`win32.rs`/`macos/gl.rs`), instead of leaking out to
/// `natives.rs`.
/// Always compiled (unlike `WindowHandle`'s GL-calling methods below) since
/// it carries no platform state — just a tag `natives.rs` maps a Fable
/// `"vertex"`/`"index"` string onto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GfxBufferKind {
    Vertex,
    Index,
}

/// Handle to a live (or already torn-down) window + GL context — the
/// runtime backing for the `Window` builtin type. A GC leaf (see
/// `Obj::Window` in `src/value.rs`): nothing GC-relevant lives inside it,
/// only OS/GL handles, same reasoning as `Obj::Worker`.
pub struct WindowHandle {
    #[cfg(any(
        all(any(feature = "gl", feature = "vulkan"), target_os = "linux"),
        all(any(feature = "gl", feature = "vulkan"), target_os = "windows"),
        all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64")
    ))]
    inner: Option<PlatformInner>,
}

impl std::fmt::Debug for WindowHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<window>")
    }
}

/// Create a window titled `title`, `w` by `h` pixels, with a GL context made
/// current. Always an `Err` without the `gl` feature or on an unsupported
/// target.
#[cfg(not(any(
    all(any(feature = "gl", feature = "vulkan"), target_os = "linux"),
    all(any(feature = "gl", feature = "vulkan"), target_os = "windows"),
    all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64")
)))]
pub fn create(_title: &str, _w: i32, _h: i32) -> Result<WindowHandle, String> {
    Err("windowing support not compiled in (build with --features gl)".to_string())
}

#[cfg(any(
    all(any(feature = "gl", feature = "vulkan"), target_os = "linux"),
    all(any(feature = "gl", feature = "vulkan"), target_os = "windows"),
    all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64")
))]
pub fn create(title: &str, w: i32, h: i32) -> Result<WindowHandle, String> {
    let inner = PlatformInner::create(title, w, h)?;
    Ok(WindowHandle { inner: Some(inner) })
}

/// Create a Metal-backed window (macOS/Apple Silicon only) — additive
/// alongside [`create`]'s OpenGL/CGL path, never a replacement (see
/// `CLAUDE.md`'s standing Metal exception). A sibling function rather than a
/// `backend` parameter on `create`: Fable has neither default parameters nor
/// overloading, so a mandatory extra argument would break every existing
/// `window.create(title, w, h)` call site for no ergonomic gain. Always an
/// `Err` without the `metal` feature or off Apple Silicon macOS.
#[cfg(not(all(feature = "metal", target_os = "macos", target_arch = "aarch64")))]
pub fn create_metal(_title: &str, _w: i32, _h: i32) -> Result<WindowHandle, String> {
    Err(
        "Metal windowing support not compiled in (build with --features metal, \
         aarch64-apple-darwin only)"
            .to_string(),
    )
}

#[cfg(all(feature = "metal", target_os = "macos", target_arch = "aarch64"))]
pub fn create_metal(title: &str, w: i32, h: i32) -> Result<WindowHandle, String> {
    let inner = macos::Inner::create_metal(title, w, h)?;
    Ok(WindowHandle { inner: Some(inner) })
}

/// Create a Vulkan-backed window (Linux/X11 and Windows) — additive
/// alongside [`create`]'s OpenGL/GLX path, never a replacement, exactly the
/// shape [`create_metal`] established on macOS (and a sibling function
/// rather than a `backend` parameter on `create` for the same reason: Fable
/// has neither default parameters nor overloading). Rides the same `vulkan`
/// cargo feature as the `gpu.run_spirv` compute backend. Always an `Err`
/// without the `vulkan` feature or off Linux — and, until the Vulkan
/// graphics arc's Phase 1 lands, a clean "not yet implemented" `Err` even
/// with it on (see `x11/vulkan.rs`).
#[cfg(not(all(feature = "vulkan", any(target_os = "linux", target_os = "windows"))))]
pub fn create_vulkan(_title: &str, _w: i32, _h: i32) -> Result<WindowHandle, String> {
    Err(
        "Vulkan windowing support not compiled in (build with --features vulkan, \
         Linux/X11 or Windows)"
            .to_string(),
    )
}

#[cfg(all(feature = "vulkan", any(target_os = "linux", target_os = "windows")))]
pub fn create_vulkan(title: &str, w: i32, h: i32) -> Result<WindowHandle, String> {
    let inner = PlatformInner::create_vulkan(title, w, h)?;
    Ok(WindowHandle { inner: Some(inner) })
}

#[cfg(any(
    all(any(feature = "gl", feature = "vulkan"), target_os = "linux"),
    all(any(feature = "gl", feature = "vulkan"), target_os = "windows"),
    all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64")
))]
impl WindowHandle {
    /// Pump the platform event queue; updates should-close/key/mouse/size
    /// state.
    pub fn poll(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.poll();
        }
    }

    /// Once `true` (the window manager's close button, `Alt+F4`/`Cmd+Q`,
    /// etc. was caught) it stays `true`. Also `true` once the window has
    /// been explicitly `close()`d.
    pub fn should_close(&self) -> bool {
        match &self.inner {
            Some(inner) => inner.should_close(),
            None => true,
        }
    }

    /// Explicit early teardown (see the module docs). Idempotent.
    pub fn close(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.teardown();
        }
    }

    pub fn key_down(&self, name: &str) -> bool {
        match &self.inner {
            Some(inner) => inner.key_down(name),
            None => false,
        }
    }

    pub fn mouse_pos(&self) -> (f64, f64) {
        match &self.inner {
            Some(inner) => inner.mouse(),
            None => (0.0, 0.0),
        }
    }

    pub fn width(&self) -> i32 {
        self.inner.as_ref().map_or(0, |i| i.width())
    }

    pub fn height(&self) -> i32 {
        self.inner.as_ref().map_or(0, |i| i.height())
    }

    /// `"opengl"`, `"metal"`, or `"vulkan"` — the one place a Fable program
    /// needs to branch when writing a single codebase that targets multiple
    /// backends, since shader *source text* (GLSL vs. MSL vs. SPIR-V) is
    /// inherently backend-specific; everything else about the `gfx` call
    /// shape is identical across backends.
    pub fn backend_name(&self) -> String {
        self.inner.as_ref().map_or("?", |i| i.backend_name()).to_string()
    }

    pub fn clear(&mut self, r: f64, g: f64, b: f64, a: f64) {
        if let Some(inner) = &mut self.inner {
            inner.clear(r as f32, g as f32, b as f32, a as f32);
        }
    }

    pub fn swap_buffers(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.swap_buffers();
        }
    }

    // -----------------------------------------------------------------
    // gfx.* (v0.8) — GL 3.3 core-profile draw calls against this window's
    // context. Each of these forwards to the matching `Inner` method (same
    // name on every platform backend, mirroring `poll`/`clear`/etc. above);
    // `natives.rs` never touches raw GL enum values or FFI types, only these
    // Fable-shaped wrappers. See `docs/SPEC.md` § 7.4 for the full contract.
    // -----------------------------------------------------------------

    /// Makes this window's GL context current on this thread. Idempotent,
    /// and the same call `clear()`/`swap_buffers()` already make internally
    /// per call — this just exposes it as its own public method so
    /// `gfx.*` natives have a window to operate against (see
    /// `Vm::gfx_current_window`).
    pub fn make_current(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.make_current();
        }
    }

    pub fn gl_compile_program(&mut self, vertex_src: &str, fragment_src: &str) -> Result<u32, String> {
        match &mut self.inner {
            Some(inner) => inner.compile_program(vertex_src, fragment_src),
            None => Err("gfx: window is closed".to_string()),
        }
    }

    /// `gfx.compile_program_spirv` (the Vulkan backend's shader input) —
    /// same shape as [`Self::gl_compile_program`]; backends that take
    /// source text return a clean redirecting `Err`.
    pub fn gl_compile_program_spirv(&mut self, vs: &[u8], fs: &[u8]) -> Result<u32, String> {
        match &mut self.inner {
            Some(inner) => inner.compile_program_spirv(vs, fs),
            None => Err("gfx: window is closed".to_string()),
        }
    }

    pub fn gl_use_program(&mut self, program: u32) {
        if let Some(inner) = &mut self.inner {
            inner.use_program(program);
        }
    }

    pub fn gl_delete_program(&mut self, program: u32) {
        if let Some(inner) = &mut self.inner {
            inner.delete_program(program);
        }
    }

    pub fn gl_create_buffer(&mut self) -> u32 {
        self.inner.as_mut().map_or(0, |inner| inner.create_buffer())
    }

    pub fn gl_delete_buffer(&mut self, buffer: u32) {
        if let Some(inner) = &mut self.inner {
            inner.delete_buffer(buffer);
        }
    }

    pub fn gl_bind_buffer(&mut self, kind: GfxBufferKind, buffer: u32) {
        if let Some(inner) = &mut self.inner {
            inner.bind_buffer(kind, buffer);
        }
    }

    pub fn gl_upload_buffer(&mut self, kind: GfxBufferKind, data: &[u8], dynamic: bool) {
        if let Some(inner) = &mut self.inner {
            inner.upload_buffer(kind, data, dynamic);
        }
    }

    pub fn gl_create_vertex_array(&mut self) -> u32 {
        self.inner.as_mut().map_or(0, |inner| inner.create_vertex_array())
    }

    pub fn gl_bind_vertex_array(&mut self, vao: u32) {
        if let Some(inner) = &mut self.inner {
            inner.bind_vertex_array(vao);
        }
    }

    pub fn gl_delete_vertex_array(&mut self, vao: u32) {
        if let Some(inner) = &mut self.inner {
            inner.delete_vertex_array(vao);
        }
    }

    pub fn gl_set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        if let Some(inner) = &mut self.inner {
            inner.set_vertex_attrib(index, size, stride, offset);
        }
    }

    pub fn gl_disable_vertex_attrib(&mut self, index: u32) {
        if let Some(inner) = &mut self.inner {
            inner.disable_vertex_attrib(index);
        }
    }

    pub fn gl_create_texture(&mut self) -> u32 {
        self.inner.as_mut().map_or(0, |inner| inner.create_texture())
    }

    pub fn gl_delete_texture(&mut self, tex: u32) {
        if let Some(inner) = &mut self.inner {
            inner.delete_texture(tex);
        }
    }

    pub fn gl_bind_texture(&mut self, tex: u32) {
        if let Some(inner) = &mut self.inner {
            inner.bind_texture(tex);
        }
    }

    pub fn gl_active_texture_unit(&mut self, unit: u32) {
        if let Some(inner) = &mut self.inner {
            inner.active_texture_unit(unit);
        }
    }

    pub fn gl_upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        if let Some(inner) = &mut self.inner {
            inner.upload_texture(data, width, height, has_alpha);
        }
    }

    pub fn gl_set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        if let Some(inner) = &mut self.inner {
            inner.set_uniform_int(program, name, v);
        }
    }

    pub fn gl_set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        if let Some(inner) = &mut self.inner {
            inner.set_uniform_float(program, name, v);
        }
    }

    pub fn gl_set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        if let Some(inner) = &mut self.inner {
            inner.set_uniform_vec2(program, name, x, y);
        }
    }

    pub fn gl_set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        if let Some(inner) = &mut self.inner {
            inner.set_uniform_vec3(program, name, x, y, z);
        }
    }

    pub fn gl_set_uniform_vec4(
        &mut self,
        program: u32,
        name: &str,
        x: f32,
        y: f32,
        z: f32,
        w: f32,
    ) {
        if let Some(inner) = &mut self.inner {
            inner.set_uniform_vec4(program, name, x, y, z, w);
        }
    }

    pub fn gl_set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        if let Some(inner) = &mut self.inner {
            inner.set_uniform_mat4(program, name, values);
        }
    }

    pub fn gl_draw_arrays(&mut self, first: i32, count: i32) {
        if let Some(inner) = &mut self.inner {
            inner.draw_arrays(first, count);
        }
    }

    pub fn gl_draw_elements(&mut self, count: i32, byte_offset: i32) {
        if let Some(inner) = &mut self.inner {
            inner.draw_elements(count, byte_offset);
        }
    }

    /// `glClearColor` + `glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT)`
    /// — unlike `clear()` above (color only), `gfx.clear` also clears depth.
    pub fn gl_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        if let Some(inner) = &mut self.inner {
            inner.gfx_clear(r, g, b, a);
        }
    }

    pub fn gl_set_depth_test(&mut self, enabled: bool) {
        if let Some(inner) = &mut self.inner {
            inner.set_depth_test(enabled);
        }
    }

    pub fn gl_viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        if let Some(inner) = &mut self.inner {
            inner.viewport(x, y, w, h);
        }
    }

    pub fn gl_read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        match &mut self.inner {
            Some(inner) => inner.read_pixels(x, y, w, h),
            None => vec![0u8; (w.max(0) as usize) * (h.max(0) as usize) * 4],
        }
    }
}

#[cfg(any(
    all(any(feature = "gl", feature = "vulkan"), target_os = "linux"),
    all(any(feature = "gl", feature = "vulkan"), target_os = "windows"),
    all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64")
))]
impl Drop for WindowHandle {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            inner.teardown();
        }
    }
}

// ---------------------------------------------------------------------------
// Feature/platform OFF: graceful stubs. `create` above always errs, so a
// `WindowHandle` is never actually constructed in this build — these method
// bodies exist only so `natives.rs`'s `call_native` compiles uniformly
// across every feature/platform combination (mirrors `src/gpu.rs`'s stubs).
// ---------------------------------------------------------------------------

#[cfg(not(any(
    all(any(feature = "gl", feature = "vulkan"), target_os = "linux"),
    all(any(feature = "gl", feature = "vulkan"), target_os = "windows"),
    all(any(feature = "gl", feature = "metal"), target_os = "macos", target_arch = "aarch64")
)))]
impl WindowHandle {
    pub fn poll(&mut self) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn should_close(&self) -> bool {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn close(&mut self) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn key_down(&self, _name: &str) -> bool {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn mouse_pos(&self) -> (f64, f64) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn width(&self) -> i32 {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn height(&self) -> i32 {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn backend_name(&self) -> String {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn clear(&mut self, _r: f64, _g: f64, _b: f64, _a: f64) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn swap_buffers(&mut self) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }

    // gfx.* (v0.8) stubs — see the doc comment on the block above. A
    // `WindowHandle` is never constructed in this build (`create` above
    // always errs), so none of these are ever actually reached; they exist
    // only so `natives.rs`'s `call_native` compiles uniformly regardless of
    // the `gl` feature/platform combo.
    pub fn make_current(&mut self) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_compile_program(&mut self, _vertex_src: &str, _fragment_src: &str) -> Result<u32, String> {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_compile_program_spirv(&mut self, _vs: &[u8], _fs: &[u8]) -> Result<u32, String> {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_use_program(&mut self, _program: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_delete_program(&mut self, _program: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_create_buffer(&mut self) -> u32 {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_delete_buffer(&mut self, _buffer: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_bind_buffer(&mut self, _kind: GfxBufferKind, _buffer: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_upload_buffer(&mut self, _kind: GfxBufferKind, _data: &[u8], _dynamic: bool) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_create_vertex_array(&mut self) -> u32 {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_bind_vertex_array(&mut self, _vao: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_delete_vertex_array(&mut self, _vao: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_vertex_attrib(&mut self, _index: u32, _size: i32, _stride: i32, _offset: i32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_disable_vertex_attrib(&mut self, _index: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_create_texture(&mut self) -> u32 {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_delete_texture(&mut self, _tex: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_bind_texture(&mut self, _tex: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_active_texture_unit(&mut self, _unit: u32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_upload_texture(&mut self, _data: &[u8], _width: i32, _height: i32, _has_alpha: bool) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_uniform_int(&mut self, _program: u32, _name: &str, _v: i32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_uniform_float(&mut self, _program: u32, _name: &str, _v: f32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_uniform_vec2(&mut self, _program: u32, _name: &str, _x: f32, _y: f32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_uniform_vec3(&mut self, _program: u32, _name: &str, _x: f32, _y: f32, _z: f32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_uniform_vec4(
        &mut self,
        _program: u32,
        _name: &str,
        _x: f32,
        _y: f32,
        _z: f32,
        _w: f32,
    ) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_uniform_mat4(&mut self, _program: u32, _name: &str, _values: &[f32; 16]) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_draw_arrays(&mut self, _first: i32, _count: i32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_draw_elements(&mut self, _count: i32, _byte_offset: i32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_clear(&mut self, _r: f32, _g: f32, _b: f32, _a: f32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_set_depth_test(&mut self, _enabled: bool) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_viewport(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn gl_read_pixels(&mut self, _x: i32, _y: i32, _w: i32, _h: i32) -> Vec<u8> {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `create_metal` degrades gracefully off Apple Silicon macOS (or with
    /// the `metal` feature disabled) — unlike the GL/Cocoa backends, this is
    /// deterministic and needs no display or hardware, so it runs in every
    /// CI environment including this Linux dev container.
    #[test]
    fn create_metal_stub_errs_off_apple_silicon_macos() {
        if cfg!(all(feature = "metal", target_os = "macos", target_arch = "aarch64")) {
            eprintln!("skipping: metal feature is actually compiled in on this target");
            return;
        }
        let err = create_metal("fable window test", 320, 240).unwrap_err();
        assert!(err.contains("Metal windowing support not compiled in"));
        assert!(err.contains("--features metal"));
    }

    /// The same graceful degradation for `create_vulkan` off Linux (or with
    /// the `vulkan` feature disabled) — deterministic, no display or
    /// hardware needed, runs in every CI environment. (With the feature on,
    /// on Linux, `x11/mod.rs`'s own test covers the Phase-0 stub `Err`.)
    #[test]
    fn create_vulkan_stub_errs_without_linux_vulkan() {
        if cfg!(all(feature = "vulkan", any(target_os = "linux", target_os = "windows"))) {
            eprintln!("skipping: vulkan windowing is actually compiled in on this target");
            return;
        }
        let err = create_vulkan("fable window test", 320, 240).unwrap_err();
        assert!(err.contains("Vulkan windowing support not compiled in"));
        assert!(err.contains("--features vulkan"));
    }
}
