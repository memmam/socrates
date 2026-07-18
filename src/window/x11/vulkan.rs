//! Vulkan backend for the `window` namespace on Linux, additive alongside
//! `gl.rs` (OpenGL/GLX) — never a replacement. A single compiled binary can
//! hold either kind of window, or both at once (`--features gl,vulkan`);
//! see `super::Inner`'s two-variant enum. Everything downstream of the
//! surface — device pick, swapchain, the offscreen stable back buffer,
//! clear/present, and the whole `gfx.*` draw-call machinery — is
//! [`Chain`], the platform-neutral core in `window/vulkan.rs` shared with
//! the Windows backend. This file owns only what is genuinely
//! X11-specific: the X window itself (a composed [`X11WindowState`],
//! exactly like `gl.rs`), the `VK_KHR_xlib_surface` instance extension,
//! and the `vkCreateXlibSurfaceKHR` call over the live display and window.
//! The lavapipe pixel asserts that gate this backend in CI are therefore
//! the proof for the shared core wherever it runs.

use std::ffi::{c_ulong, c_void};
use std::ptr;
use std::sync::atomic::Ordering;

use super::shared::{
    record_x_error, X11WindowState, XDefaultDepth, XDefaultScreen, XDefaultVisual, XOpenDisplay,
    XSetErrorHandler, XSync, X_FALSE, X_PROTOCOL_ERROR,
};
use crate::vk::{loader_gipa, VkInstance, VkResult, VK_SUCCESS};
use crate::window::vulkan::{vkload, Chain, VkSurfaceKhr};

const ST_XLIB_SURFACE_CREATE_INFO_KHR: i32 = 1000004000;

#[repr(C)]
struct VkXlibSurfaceCreateInfoKhr {
    s_type: i32,
    p_next: *const c_void,
    flags: u32,
    dpy: *mut c_void,
    window: c_ulong,
}
type FnCreateXlibSurfaceKhr = unsafe extern "system" fn(
    VkInstance,
    *const VkXlibSurfaceCreateInfoKhr,
    *const c_void,
    *mut VkSurfaceKhr,
) -> VkResult;

/// The Vulkan half of a `WindowHandle` on Linux — an [`X11WindowState`]
/// (the window + event pump, composed from `shared.rs`, exactly like
/// `gl.rs`) plus the shared [`Chain`].
pub struct Inner {
    x11: X11WindowState,
    chain: Chain,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        let Some(gipa) = loader_gipa() else {
            return Err(
                "window.create_vulkan: dlopen(\"libvulkan.so.1\") failed — no Vulkan loader \
                 installed?"
                    .to_string(),
            );
        };

        // Safety: X half mirrors gl.rs's create exactly (shared machinery,
        // same async-protocol-error watch discipline — restore only after
        // every X call, teardown's included, is done); the Vulkan half is
        // checked call by call inside [`Chain::create`], which unwinds
        // everything it created before a failure.
        unsafe {
            let display = XOpenDisplay(ptr::null());
            if display.is_null() {
                return Err(
                    "window.create_vulkan: XOpenDisplay failed (no X server / $DISPLAY not \
                     set?)"
                        .to_string(),
                );
            }
            let screen = XDefaultScreen(display);

            X_PROTOCOL_ERROR.store(false, Ordering::SeqCst);
            let prev_handler = XSetErrorHandler(Some(record_x_error));

            let x11 = X11WindowState::create_window(
                display,
                screen,
                XDefaultVisual(display, screen),
                XDefaultDepth(display, screen),
                title,
                w,
                h,
            );

            XSync(display, X_FALSE);
            if X_PROTOCOL_ERROR.load(Ordering::SeqCst) {
                x11.teardown();
                XSetErrorHandler(prev_handler);
                return Err(
                    "window.create_vulkan: an X protocol error occurred while creating the \
                     window"
                        .to_string(),
                );
            }

            let chain = Chain::create(
                gipa,
                "VK_KHR_xlib_surface",
                "an X11 surface",
                |gipa, instance| {
                    let create_xlib_surface = vkload!(
                        gipa,
                        instance,
                        "vkCreateXlibSurfaceKHR",
                        FnCreateXlibSurfaceKhr
                    );
                    let xci = VkXlibSurfaceCreateInfoKhr {
                        s_type: ST_XLIB_SURFACE_CREATE_INFO_KHR,
                        p_next: ptr::null(),
                        flags: 0,
                        dpy: x11.display as *mut c_void,
                        window: x11.window,
                    };
                    let mut surface: VkSurfaceKhr = 0;
                    let r = create_xlib_surface(instance, &xci, ptr::null(), &mut surface);
                    if r != VK_SUCCESS {
                        return Err(format!(
                            "window.create_vulkan: vkCreateXlibSurfaceKHR failed ({r})"
                        ));
                    }
                    Ok(surface)
                },
                (w, h),
            );
            match chain {
                Ok(chain) => {
                    XSetErrorHandler(prev_handler);
                    Ok(Inner { x11, chain })
                }
                Err(e) => {
                    // `Chain::create` has already torn down the partial
                    // Vulkan chain on failure — the X state and handler
                    // restore are this shim's to unwind, like gl.rs.
                    x11.teardown();
                    XSetErrorHandler(prev_handler);
                    Err(e)
                }
            }
        }
    }

    pub fn poll(&mut self) {
        self.x11.poll();
    }

    pub fn key_down(&self, name: &str) -> bool {
        self.x11.key_down(name)
    }

    pub fn mouse(&self) -> (f64, f64) {
        self.x11.mouse
    }
    pub fn width(&self) -> i32 {
        self.x11.width
    }
    pub fn height(&self) -> i32 {
        self.x11.height
    }
    pub fn should_close(&self) -> bool {
        self.x11.should_close
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
        self.chain.swap_buffers((self.x11.width, self.x11.height));
    }

    /// Idempotent-by-construction teardown (consumes `self`): Vulkan chain
    /// in reverse creation order, then the X11 half — the same split
    /// `gl.rs`'s teardown has (context first, then
    /// [`X11WindowState::teardown`]).
    pub fn teardown(self) {
        self.chain.destroy();
        self.x11.teardown();
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
    use std::ffi::c_int;

    /// The Phase 1 correctness gate, with real pixels as ground truth:
    /// create a Vulkan window, clear the back buffer to a known color,
    /// present it, and read the pixel back out of the X window itself via
    /// XGetImage. UNORM linearity is asserted exactly ([255, 128, 0] from
    /// a (1.0, 0.5, 0.0) clear — an sRGB-format regression would read
    /// [255, 188, 0]). Skips gracefully without a display or a Vulkan
    /// device (headless environments without lavapipe).
    #[test]
    fn create_clear_present_pixel_roundtrip() {
        if std::env::var_os("DISPLAY").is_none() {
            eprintln!("skipping: $DISPLAY not set");
            return;
        }
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

        // Ground truth: the presented color must be in the X window.
        // XGetImage declared test-locally (production code never reads the
        // window back — Phase 2's read_pixels reads the offscreen target).
        #[repr(C)]
        struct XImagePrefix {
            width: c_int,
            height: c_int,
            xoffset: c_int,
            format: c_int,
            data: *mut u8,
        }
        extern "C" {
            fn XGetImage(
                d: *mut super::super::shared::Display,
                drawable: c_ulong,
                x: c_int,
                y: c_int,
                w: std::ffi::c_uint,
                h: std::ffi::c_uint,
                plane_mask: c_ulong,
                format: c_int,
            ) -> *mut XImagePrefix;
        }
        unsafe {
            // Two things make a single read-after-present unreliable, both
            // seen in practice: (a) Mesa's X11 WSI queues FIFO presents on
            // an internal thread, so vkQueuePresentKHR returning — even
            // followed by XSync — doesn't mean the XPutImage has landed;
            // (b) the test harness runs the GL window smoke concurrently,
            // and under Xvfb's WM-less stacking both 320x240 windows sit at
            // (0, 0) — XGetImage on an occluded (or freshly re-exposed)
            // X11 window reads the occluder's pixels or stale content,
            // since X never repaints exposed regions for you. So each
            // bounded retry re-presents the frame first (a real program's
            // frame loop self-heals exposure damage the same way) and
            // asserts the exact value only at timeout.
            let mut last = (0u8, 0u8, 0u8);
            for _ in 0..80 {
                inner.clear(1.0, 0.5, 0.0, 1.0);
                inner.swap_buffers();
                XSync(inner.x11.display, X_FALSE);
                let img = XGetImage(
                    inner.x11.display,
                    inner.x11.window,
                    160,
                    120,
                    1,
                    1,
                    !0,
                    2, // ZPixmap
                );
                assert!(!img.is_null(), "XGetImage failed");
                // 24-bit ZPixmap on little-endian: bytes are B, G, R.
                let d = (*img).data;
                last = (*d.add(2), *d.add(1), *d);
                if last == (255, 128, 0) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            assert_eq!(
                last,
                (255, 128, 0),
                "presented pixel is not the linear orange that was cleared (RGB shown)"
            );
        }
        inner.teardown();
    }
}
