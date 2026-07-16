//! Windowing for the `window` builtin namespace — the GLFW-equivalent piece
//! of the native-OpenGL roadmap (`std.glm`, v0.8, already shipped the math
//! side; the Linux/X11/GLX backend shipped after that). Scope for now:
//! window creation, event polling, keyboard/mouse state, and a trivial
//! clear-color + swap-buffers, enough to prove the whole pipe end-to-end. A
//! general `gl` draw-call namespace (a GL function-pointer loader with the
//! usual few dozen entries) is a later PR.
//!
//! This module is **always compiled**; only its internals are gated on the
//! `gl` cargo feature, mirroring `src/gpu.rs` exactly. The feature adds
//! **zero** Cargo dependencies — it is raw FFI to system libraries (X11/
//! Win32 linked normally, GL/GLX/WGL resolved with `dlopen`+`dlsym` or
//! `LoadLibrary`+`GetProcAddress` at runtime, Cocoa/NSOpenGL messaged via
//! `objc_msgSend`) — so `cargo tree -e normal` stays a single line with or
//! without it.
//!
//! Three platform backends, each behind its own submodule: [`x11`] (Linux,
//! any arch), [`win32`] (Windows, any arch), [`macos`] (macOS — gated on
//! `target_arch = "aarch64"` **in addition to** `target_os`, not just by
//! convention: it sidesteps the struct-return `objc_msgSend_stret` ABI split
//! that only exists on x86_64, so an accidental `x86_64-apple-darwin` build
//! must not silently compile the same code in and hit that split at
//! runtime). Exactly one backend is ever compiled in, aliased to
//! `PlatformInner` below, so
//! [`WindowHandle`]'s method bodies are written once and never see which
//! platform they're actually running on. On any other target (or with the
//! feature off), [`create`] degrades gracefully to `Err`, the same way
//! `gpu::run` does without the `gpu` feature.
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

#[cfg(all(feature = "gl", target_os = "linux"))]
pub mod x11;
#[cfg(all(feature = "gl", target_os = "linux"))]
use x11::Inner as PlatformInner;

#[cfg(all(feature = "gl", target_os = "windows"))]
pub mod win32;
#[cfg(all(feature = "gl", target_os = "windows"))]
use win32::Inner as PlatformInner;

#[cfg(all(feature = "gl", target_os = "macos", target_arch = "aarch64"))]
pub mod macos;
#[cfg(all(feature = "gl", target_os = "macos", target_arch = "aarch64"))]
use macos::Inner as PlatformInner;

/// Handle to a live (or already torn-down) window + GL context — the
/// runtime backing for the `Window` builtin type. A GC leaf (see
/// `Obj::Window` in `src/value.rs`): nothing GC-relevant lives inside it,
/// only OS/GL handles, same reasoning as `Obj::Worker`.
pub struct WindowHandle {
    #[cfg(all(
        feature = "gl",
        any(target_os = "linux", target_os = "windows", all(target_os = "macos", target_arch = "aarch64"))
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
#[cfg(not(all(
    feature = "gl",
    any(target_os = "linux", target_os = "windows", all(target_os = "macos", target_arch = "aarch64"))
)))]
pub fn create(_title: &str, _w: i32, _h: i32) -> Result<WindowHandle, String> {
    Err("windowing support not compiled in (build with --features gl)".to_string())
}

#[cfg(all(
    feature = "gl",
    any(target_os = "linux", target_os = "windows", all(target_os = "macos", target_arch = "aarch64"))
))]
pub fn create(title: &str, w: i32, h: i32) -> Result<WindowHandle, String> {
    let inner = PlatformInner::create(title, w, h)?;
    Ok(WindowHandle { inner: Some(inner) })
}

#[cfg(all(
    feature = "gl",
    any(target_os = "linux", target_os = "windows", all(target_os = "macos", target_arch = "aarch64"))
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
            Some(inner) => inner.should_close,
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
            Some(inner) => inner.mouse,
            None => (0.0, 0.0),
        }
    }

    pub fn width(&self) -> i32 {
        self.inner.as_ref().map_or(0, |i| i.width)
    }

    pub fn height(&self) -> i32 {
        self.inner.as_ref().map_or(0, |i| i.height)
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
}

#[cfg(all(
    feature = "gl",
    any(target_os = "linux", target_os = "windows", all(target_os = "macos", target_arch = "aarch64"))
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

#[cfg(not(all(
    feature = "gl",
    any(target_os = "linux", target_os = "windows", all(target_os = "macos", target_arch = "aarch64"))
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
    pub fn clear(&mut self, _r: f64, _g: f64, _b: f64, _a: f64) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
    pub fn swap_buffers(&mut self) {
        unreachable!("WindowHandle is never constructed without a compiled-in backend")
    }
}
