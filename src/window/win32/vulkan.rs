//! The Vulkan half of the Windows window backend — **Phase 0
//! scaffolding**: `create` returns a clean `Err`, so no Vulkan-backed
//! window can exist yet and [`Inner`] is deliberately *uninhabited* (an
//! empty enum). Every `super::Inner` dispatch arm for the `Vulkan` variant
//! is therefore statically unreachable (`match *never {}`), with no stub
//! methods to maintain — they become real forwards when the WSI phase
//! lands, exactly as `x11/vulkan.rs` grew from its own Phase-0 stub.
//!
//! The WSI phase (next PR) mirrors `x11/vulkan.rs`: a
//! `VK_KHR_win32_surface` instance extension + `vkCreateWin32SurfaceKHR`
//! over the shared `crate::vk` primitive layer, the same offscreen
//! stable-back-buffer presentation model, and the same B8G8R8A8_UNORM
//! format preference.

/// Uninhabited until the WSI phase: no value of this type can exist, which
/// is exactly the Phase-0 contract ([`Inner::create`] always `Err`s).
pub enum Inner {}

impl Inner {
    pub fn create(_title: &str, _w: i32, _h: i32) -> Result<Inner, String> {
        Err(
            "window.create_vulkan: the Win32 Vulkan window surface is not yet implemented \
             (scaffolding only — the WSI phase is next on CLAUDE.md's native-backend roadmap)"
                .to_string(),
        )
    }
}
