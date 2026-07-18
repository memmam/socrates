//! Vulkan backend for the `window` namespace on Windows, additive
//! alongside `gl.rs` (OpenGL/WGL) — never a replacement. A single compiled
//! binary can hold either kind of window, or both at once (`--features
//! gl,vulkan`); see `super::Inner`'s two-variant enum. Everything
//! downstream of the surface — device pick, swapchain, the offscreen
//! stable back buffer, clear/present, and the whole `gfx.*` draw-call
//! surface — is [`Chain`], the platform-neutral core in
//! `window/vulkan.rs` shared with the X11 backend, whose lavapipe pixel
//! asserts in CI prove the exact code this backend runs. This file owns
//! only what is genuinely Win32-specific: the window itself (a composed
//! [`Win32WindowState`], exactly like `gl.rs`), the
//! `VK_KHR_win32_surface` instance extension, and the
//! `vkCreateWin32SurfaceKHR` call over the module `HINSTANCE` and the
//! window's `HWND`. The loader comes from [`crate::vk::loader_gipa`]
//! (`LoadLibraryA("vulkan-1.dll")` here) — windows-latest CI runners ship
//! the loader DLL but no ICD behind it, so every failure path is a clean
//! prefixed `Err` (CI echoes the graceful line; a Vulkan-capable machine
//! exercises the real path, with `read_pixels` as the byte-exact
//! in-program gate, as on macOS).

use std::ffi::c_void;
use std::ptr;

use super::shared::{GetModuleHandleW, Win32WindowState};
use crate::vk::{loader_gipa, VkInstance, VkResult, VK_SUCCESS};
use crate::window::vulkan::{vkload, Chain, VkSurfaceKhr};

const ST_WIN32_SURFACE_CREATE_INFO_KHR: i32 = 1000009000;

#[repr(C)]
struct VkWin32SurfaceCreateInfoKhr {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    hinstance: *mut c_void,
    hwnd: *mut c_void,
}
type FnCreateWin32SurfaceKhr = unsafe extern "system" fn(
    VkInstance,
    *const VkWin32SurfaceCreateInfoKhr,
    *const c_void,
    *mut VkSurfaceKhr,
) -> VkResult;

/// The Vulkan half of a `WindowHandle` on Windows — a
/// [`Win32WindowState`] (the window + message pump, composed from
/// `shared.rs`, exactly like `gl.rs`) plus the shared [`Chain`].
pub struct Inner {
    win32: Win32WindowState,
    chain: Chain,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        let Some(gipa) = loader_gipa() else {
            return Err(
                "window.create_vulkan: no Vulkan loader (vulkan-1.dll) on this system"
                    .to_string(),
            );
        };

        // Safety: the Win32 half mirrors gl.rs's create exactly (shared
        // machinery); the Vulkan half is checked call by call inside
        // [`Chain::create`], which unwinds everything it created before a
        // failure.
        unsafe {
            let win32 = Win32WindowState::create_window("window.create_vulkan", title, w, h)?;
            let chain = Chain::create(
                gipa,
                "VK_KHR_win32_surface",
                "a Win32 surface",
                |gipa, instance| {
                    let create_win32_surface = vkload!(
                        gipa,
                        instance,
                        "vkCreateWin32SurfaceKHR",
                        FnCreateWin32SurfaceKhr
                    );
                    let wci = VkWin32SurfaceCreateInfoKhr {
                        s_type: ST_WIN32_SURFACE_CREATE_INFO_KHR,
                        p_next: ptr::null(),
                        flags: 0,
                        hinstance: GetModuleHandleW(ptr::null()),
                        hwnd: win32.hwnd,
                    };
                    let mut surface: VkSurfaceKhr = 0;
                    let r = create_win32_surface(instance, &wci, ptr::null(), &mut surface);
                    if r != VK_SUCCESS {
                        return Err(format!(
                            "window.create_vulkan: vkCreateWin32SurfaceKHR failed ({r})"
                        ));
                    }
                    Ok(surface)
                },
                (w, h),
            );
            match chain {
                Ok(chain) => {
                    let inner = Inner { win32, chain };
                    inner.win32.show();
                    Ok(inner)
                }
                Err(e) => {
                    // `Chain::create` has already torn down the partial
                    // Vulkan chain on failure — the Win32 state is this
                    // shim's to unwind, like gl.rs.
                    win32.teardown();
                    Err(e)
                }
            }
        }
    }

    pub fn poll(&mut self) {
        self.win32.poll();
    }

    pub fn key_down(&self, name: &str) -> bool {
        self.win32.key_down(name)
    }

    pub fn mouse(&self) -> (f64, f64) {
        self.win32.mouse
    }
    pub fn width(&self) -> i32 {
        self.win32.width
    }
    pub fn height(&self) -> i32 {
        self.win32.height
    }
    pub fn should_close(&self) -> bool {
        self.win32.should_close
    }

    /// This half is always Vulkan — `super::Inner`'s GL variant reports
    /// its own name.
    pub fn backend_name(&self) -> &'static str {
        "vulkan"
    }

    /// No-op on Vulkan (there is no thread-bound "current context" to
    /// assert the way GLX/CGL need) — exists so `win.make_current()` keeps
    /// its cross-backend meaning: "make this the window `gfx.*` targets"
    /// (the VM-level current-window registration happens in `natives.rs`,
    /// backend-independently).
    pub fn make_current(&mut self) {}

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.chain.clear(r, g, b, a);
    }

    pub fn swap_buffers(&mut self) {
        self.chain
            .swap_buffers((self.win32.width, self.win32.height));
    }

    /// Idempotent-by-construction teardown (consumes `self`): Vulkan chain
    /// in reverse creation order, then the Win32 half — the same split
    /// `gl.rs`'s teardown has (context first, then
    /// [`Win32WindowState::teardown`]).
    pub fn teardown(self) {
        self.chain.destroy();
        self.win32.teardown();
    }

    // The gfx.* draw-call surface, forwarded verbatim to the shared core.

    pub fn compile_program_spirv(&mut self, vs: &[u8], fs: &[u8]) -> Result<u32, String> {
        self.chain.compile_program_spirv(vs, fs)
    }
    pub fn use_program(&mut self, program: u32) {
        self.chain.use_program(program);
    }
    pub fn delete_program(&mut self, program: u32) {
        self.chain.delete_program(program);
    }
    pub fn create_buffer(&mut self) -> u32 {
        self.chain.create_buffer()
    }
    pub fn delete_buffer(&mut self, buffer: u32) {
        self.chain.delete_buffer(buffer);
    }
    pub fn bind_buffer(&mut self, kind: crate::window::GfxBufferKind, buffer: u32) {
        self.chain.bind_buffer(kind, buffer);
    }
    pub fn upload_buffer(&mut self, kind: crate::window::GfxBufferKind, data: &[u8], dynamic: bool) {
        self.chain.upload_buffer(kind, data, dynamic);
    }
    pub fn create_vertex_array(&mut self) -> u32 {
        self.chain.create_vertex_array()
    }
    pub fn bind_vertex_array(&mut self, vao: u32) {
        self.chain.bind_vertex_array(vao);
    }
    pub fn delete_vertex_array(&mut self, vao: u32) {
        self.chain.delete_vertex_array(vao);
    }
    pub fn set_vertex_attrib(&mut self, index: u32, size: i32, stride: i32, offset: i32) {
        self.chain.set_vertex_attrib(index, size, stride, offset);
    }
    pub fn disable_vertex_attrib(&mut self, index: u32) {
        self.chain.disable_vertex_attrib(index);
    }
    pub fn create_texture(&mut self) -> u32 {
        self.chain.create_texture()
    }
    pub fn delete_texture(&mut self, tex: u32) {
        self.chain.delete_texture(tex);
    }
    pub fn bind_texture(&mut self, tex: u32) {
        self.chain.bind_texture(tex);
    }
    pub fn active_texture_unit(&mut self, unit: u32) {
        self.chain.active_texture_unit(unit);
    }
    pub fn upload_texture(&mut self, data: &[u8], width: i32, height: i32, has_alpha: bool) {
        self.chain.upload_texture(data, width, height, has_alpha);
    }
    pub fn set_uniform_int(&mut self, program: u32, name: &str, v: i32) {
        self.chain.set_uniform_int(program, name, v);
    }
    pub fn set_uniform_float(&mut self, program: u32, name: &str, v: f32) {
        self.chain.set_uniform_float(program, name, v);
    }
    pub fn set_uniform_vec2(&mut self, program: u32, name: &str, x: f32, y: f32) {
        self.chain.set_uniform_vec2(program, name, x, y);
    }
    pub fn set_uniform_vec3(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32) {
        self.chain.set_uniform_vec3(program, name, x, y, z);
    }
    pub fn set_uniform_vec4(&mut self, program: u32, name: &str, x: f32, y: f32, z: f32, w: f32) {
        self.chain.set_uniform_vec4(program, name, x, y, z, w);
    }
    pub fn set_uniform_mat4(&mut self, program: u32, name: &str, values: &[f32; 16]) {
        self.chain.set_uniform_mat4(program, name, values);
    }
    pub fn draw_arrays(&mut self, first: i32, count: i32) {
        self.chain.draw_arrays(first, count);
    }
    pub fn draw_elements(&mut self, count: i32, byte_offset: i32) {
        self.chain.draw_elements(count, byte_offset);
    }
    pub fn gfx_clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.chain.gfx_clear(r, g, b, a);
    }
    pub fn set_depth_test(&mut self, enabled: bool) {
        self.chain.set_depth_test(enabled);
    }
    pub fn viewport(&mut self, x: i32, y: i32, w: i32, h: i32) {
        self.chain.viewport(x, y, w, h);
    }
    pub fn read_pixels(&mut self, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        self.chain.read_pixels(x, y, w, h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end smoke: create a Vulkan window (loader → instance →
    /// surface → device → swapchain → offscreen), clear, present, pump
    /// events, then `gfx_clear` + `read_pixels` as the byte-exact pixel
    /// gate (the x11 backend's XGetImage roundtrip has no portable Win32
    /// equivalent — reading the shared core's own offscreen target is the
    /// macOS precedent, and the expected bytes are the ones lavapipe
    /// produces for this exact sequence through this exact code). Skips
    /// gracefully when anything in the chain is unavailable (no loader,
    /// no device — headless CI machines vary), so every build shape is
    /// deterministic; on a Vulkan-capable Windows machine this exercises
    /// the whole pipe for real.
    #[test]
    fn create_clear_present_smoke() {
        let mut inner = match Inner::create("socrates vulkan window test", 320, 240) {
            Ok(inner) => inner,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };
        assert_eq!(inner.width(), 320);
        assert_eq!(inner.height(), 240);
        inner.clear(1.0, 0.5, 0.0, 1.0);
        inner.swap_buffers();
        inner.poll();
        assert!(!inner.should_close());
        inner.gfx_clear(1.0, 0.5, 0.0, 1.0);
        assert_eq!(inner.read_pixels(160, 120, 1, 1), [255, 128, 0, 255]);
        inner.teardown();
    }
}
