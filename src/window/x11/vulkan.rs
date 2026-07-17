//! Vulkan backend for the `window` namespace on Linux, additive alongside
//! `gl.rs` (OpenGL/GLX) — never a replacement. A single compiled binary can
//! hold either kind of window, or both at once (`--features gl,vulkan`);
//! see `super::Inner`'s two-variant enum, which is the only place either
//! backend's concrete type is named. The exact analog of the Metal arc's
//! `macos/metal.rs`, riding the same `vulkan` cargo feature the compute
//! backend (`crate::vk`, `gpu.run_spirv`) already ships under — raw
//! `dlopen("libvulkan.so.1")` FFI, zero Cargo dependencies.
//!
//! **Phase 0 (this commit): scaffolding only.** [`Inner::create`] returns a
//! clean `Err` — no instance/surface/swapchain plumbing yet. This exists so
//! `window.create_vulkan` is wired end-to-end and CI can confirm
//! `--features vulkan` and `--features gl,vulkan` both build/link/test
//! clean before any real Vulkan windowing FFI lands. Device/swapchain
//! setup, real draw calls, and the `gfx.*`-backing methods are follow-up
//! commits (see the Vulkan graphics arc's phasing in the plan file) —
//! unlike Metal, developable and testable locally: Mesa's lavapipe gives
//! this container a software Vulkan device, and Xvfb an X server.

pub struct Inner;

impl Inner {
    pub fn create(_title: &str, _w: i32, _h: i32) -> Result<Inner, String> {
        Err("window.create_vulkan: Vulkan window backend not yet implemented \
             (instance/surface/swapchain plumbing lands in a follow-up commit)"
            .to_string())
    }
}
