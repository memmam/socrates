//! Windows/Win32/WGL backend for the `window` namespace.
//!
//! **Linking strategy** (deliberate, and different from Linux's): every
//! library this module needs — `user32`, `gdi32`, `opengl32` — ships on
//! every Windows install with no separate "dev package" step (unlike Linux,
//! where GL dev headers/libs vary a lot across distros), so there is no
//! `dlopen`/`dlsym` dance here: `user32`/`gdi32`/`opengl32` are linked
//! normally, and `wglCreateContext`/`wglMakeCurrent`/`wglDeleteContext` are
//! declared as plain `extern "system"` items against `opengl32.dll`,
//! mirroring how `x11.rs` links `libX11` normally while only `libGL` is
//! resolved dynamically (the reasons for dynamic GL resolution there don't
//! apply on Windows: `opengl32.dll` is a guaranteed system component).
//!
//! Struct layouts and function prototypes below are drawn from the frozen
//! Win32 ABI (`winuser.h`/`wingdi.h`, unrevised since Windows NT 3.1/95) —
//! this session's egress policy blocked a direct fetch of the MSDN/header
//! text, so field order/values were cross-corroborated via web search
//! against multiple independent sources rather than read from a single raw
//! header, the same caveat `x11.rs`'s module docs note for its GLX tokens.
//!
//! **The WNDPROC / `GWLP_USERDATA` pattern.** Unlike Xlib (poll-based, no
//! callback), Win32 delivers window messages through a C callback
//! (`WNDPROC`) that carries no closure capture. The standard idiom — used
//! here — is to heap-allocate the `Inner`'s mutable event state in a `Box`,
//! stash the raw pointer in the window's `GWLP_USERDATA` slot right after
//! `CreateWindowExW` returns a valid `HWND`, and have `wndproc` recover it
//! (null-checked: it's `0` for the handful of messages that arrive before
//! that `SetWindowLongPtrW` call, e.g. `WM_NCCREATE`/`WM_CREATE`) to mutate
//! through the raw pointer. This is `Inner`'s only unusual ownership
//! wrinkle relative to `x11.rs`: `Inner` itself stays a single, directly
//! owned struct (matching `x11.rs`'s shape), but the fields `wndproc` needs
//! to touch (should_close/mouse/width/height/pressed keys) are boxed
//! separately so a raw pointer to them can outlive the `Inner::create` call
//! and be recovered from the window later. `Inner::teardown` frees that
//! box, mirroring `x11.rs`'s single-owner teardown discipline.

// This module's type names (`HWND`, `LPCWSTR`, `WNDPROC`, ...) match Win32's
// own naming exactly, on purpose — same call `x11.rs` makes for `XID`.
#![allow(clippy::upper_case_acronyms)]

use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr;

// ---------------------------------------------------------------------------
// Win32 types (winuser.h / windef.h / wingdi.h) — field layouts as documented
// in the task brief, cross-corroborated via web search (see module docs).
// ---------------------------------------------------------------------------

type HWND = *mut c_void;
type HDC = *mut c_void;
type HGLRC = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HMODULE = *mut c_void;
type LPCWSTR = *const u16;
type LPVOID = *mut c_void;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>;

/// `windef.h`.
#[repr(C)]
#[derive(Clone, Copy)]
struct POINT {
    x: i32,
    y: i32,
}

/// `winuser.h`. Field order confirmed via web search (see module docs).
#[repr(C)]
struct WndClassExW {
    cb_size: u32,
    style: u32,
    lpfn_wnd_proc: WNDPROC,
    cb_cls_extra: i32,
    cb_wnd_extra: i32,
    h_instance: HINSTANCE,
    h_icon: HICON,
    h_cursor: HCURSOR,
    h_background: HBRUSH,
    lpsz_menu_name: LPCWSTR,
    lpsz_class_name: LPCWSTR,
    h_icon_sm: HICON,
}

/// `winuser.h`. Six fields on non-Mac builds (the historical `lPrivate`
/// member lives behind `#ifdef _MAC`, absent on Win32).
#[repr(C)]
struct Msg {
    hwnd: HWND,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    pt: POINT,
}

/// `wingdi.h`, 26 fields. `DWORD` is fixed 32-bit even on 64-bit Windows
/// (unlike POSIX `c_ulong`), so every `DWORD`/`WORD`/`BYTE` field below maps
/// to `u32`/`u16`/`u8` respectively, never a `c_long`-family type.
#[repr(C)]
struct PixelFormatDescriptor {
    n_size: u16,
    n_version: u16,
    dw_flags: u32,
    i_pixel_type: u8,
    c_color_bits: u8,
    c_red_bits: u8,
    c_red_shift: u8,
    c_green_bits: u8,
    c_green_shift: u8,
    c_blue_bits: u8,
    c_blue_shift: u8,
    c_alpha_bits: u8,
    c_alpha_shift: u8,
    c_accum_bits: u8,
    c_accum_red_bits: u8,
    c_accum_green_bits: u8,
    c_accum_blue_bits: u8,
    c_accum_alpha_bits: u8,
    c_depth_bits: u8,
    c_stencil_bits: u8,
    c_aux_buffers: u8,
    i_layer_type: u8,
    b_reserved: u8,
    dw_layer_mask: u32,
    dw_visible_mask: u32,
    dw_damage_mask: u32,
}

// winuser.h / wingdi.h constants (values as documented in the task brief).
const CW_USEDEFAULT: i32 = 0x8000_0000u32 as i32;
const WS_OVERLAPPEDWINDOW: u32 = 0x00CF_0000;
const SW_SHOW: i32 = 5;
const PM_REMOVE: u32 = 0x0001;
const GWLP_USERDATA: i32 = -21;

const WM_DESTROY: u32 = 0x0002;
const WM_SIZE: u32 = 0x0005;
const WM_CLOSE: u32 = 0x0010;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_MOUSEMOVE: u32 = 0x0200;

const PFD_DRAW_TO_WINDOW: u32 = 0x0000_0004;
const PFD_SUPPORT_OPENGL: u32 = 0x0000_0020;
const PFD_DOUBLEBUFFER: u32 = 0x0000_0001;
const PFD_TYPE_RGBA: u8 = 0;
const PFD_MAIN_PLANE: u8 = 0;

const GL_COLOR_BUFFER_BIT: u32 = 0x0000_4000;

const IDC_ARROW: usize = 32512;

#[link(name = "user32")]
extern "system" {
    fn RegisterClassExW(lpwcx: *const WndClassExW) -> u16;
    fn CreateWindowExW(
        dw_ex_style: u32,
        lp_class_name: LPCWSTR,
        lp_window_name: LPCWSTR,
        dw_style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        h_wnd_parent: HWND,
        h_menu: HMENU,
        h_instance: HINSTANCE,
        lp_param: LPVOID,
    ) -> HWND;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w_param: usize, l_param: isize) -> isize;
    fn DestroyWindow(hwnd: HWND) -> i32;
    fn ShowWindow(hwnd: HWND, n_cmd_show: i32) -> i32;
    fn UpdateWindow(hwnd: HWND) -> i32;
    fn GetDC(hwnd: HWND) -> HDC;
    fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
    fn PeekMessageW(
        lp_msg: *mut Msg,
        h_wnd: HWND,
        w_msg_filter_min: u32,
        w_msg_filter_max: u32,
        w_remove_msg: u32,
    ) -> i32;
    fn TranslateMessage(lp_msg: *const Msg) -> i32;
    fn DispatchMessageW(lp_msg: *const Msg) -> isize;
    fn PostQuitMessage(n_exit_code: i32);
    fn LoadCursorW(h_instance: HINSTANCE, lp_cursor_name: usize) -> HCURSOR;
    fn GetModuleHandleW(lp_module_name: LPCWSTR) -> HMODULE;
    fn SetWindowLongPtrW(hwnd: HWND, n_index: i32, dw_new_long: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: HWND, n_index: i32) -> isize;
}

#[link(name = "gdi32")]
extern "system" {
    fn ChoosePixelFormat(hdc: HDC, ppfd: *const PixelFormatDescriptor) -> i32;
    fn SetPixelFormat(hdc: HDC, i_pixel_format: i32, ppfd: *const PixelFormatDescriptor) -> i32;
    fn SwapBuffers(hdc: HDC) -> i32;
}

#[link(name = "opengl32")]
extern "system" {
    fn wglCreateContext(hdc: HDC) -> HGLRC;
    fn wglMakeCurrent(hdc: HDC, hglrc: HGLRC) -> i32;
    fn wglDeleteContext(hglrc: HGLRC) -> i32;
    fn wglGetCurrentContext() -> HGLRC;
    fn glClearColor(r: f32, g: f32, b: f32, a: f32);
    fn glClear(mask: u32);
}

/// Encode a `&str` as a NUL-terminated UTF-16 buffer for the `...W` Win32
/// entry points (`CreateWindowExW` et al. take `LPCWSTR`, not `LPCSTR`).
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The mutable event-driven state `wndproc` needs to reach through
/// `GWLP_USERDATA` (see the module docs). Boxed separately from `Inner` so a
/// raw pointer to it can be installed on the window before `Inner::create`
/// returns and recovered from inside the callback later.
struct SharedState {
    pressed: HashSet<u32>,
    mouse: (f64, f64),
    width: i32,
    height: i32,
    should_close: bool,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w_param: usize, l_param: isize) -> isize {
    // Safety: `GWLP_USERDATA` holds either `0` (messages that can arrive
    // before `Inner::create` installs it, e.g. `WM_NCCREATE`/`WM_CREATE`) or
    // a valid `*mut SharedState` produced by `Box::into_raw` in
    // `Inner::create` and freed only in `Inner::teardown`, so it is always
    // either null or live for the window's whole lifetime.
    let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SharedState;
    if state_ptr.is_null() {
        return DefWindowProcW(hwnd, msg, w_param, l_param);
    }
    let state = &mut *state_ptr;
    match msg {
        WM_KEYDOWN => {
            state.pressed.insert(w_param as u32);
            0
        }
        WM_KEYUP => {
            state.pressed.remove(&(w_param as u32));
            0
        }
        WM_MOUSEMOVE => {
            // Low-order/high-order words of `lParam`, sign-extended as
            // Win32 client coordinates are: negative just off the top/left
            // edge is possible and meaningful.
            let x = (l_param & 0xFFFF) as i16 as f64;
            let y = ((l_param >> 16) & 0xFFFF) as i16 as f64;
            state.mouse = (x, y);
            0
        }
        WM_SIZE => {
            let width = (l_param & 0xFFFF) as i16 as i32;
            let height = ((l_param >> 16) & 0xFFFF) as i16 as i32;
            state.width = width;
            state.height = height;
            0
        }
        WM_CLOSE => {
            state.should_close = true;
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, w_param, l_param),
    }
}

/// The real guts of a `WindowHandle` (see `src/window/mod.rs`) — a Win32
/// window plus a current-capable WGL context.
pub struct Inner {
    hwnd: HWND,
    hdc: HDC,
    hglrc: HGLRC,
    state: *mut SharedState,
    pub mouse: (f64, f64),
    pub width: i32,
    pub height: i32,
    pub should_close: bool,
}

impl Inner {
    pub fn create(title: &str, w: i32, h: i32) -> Result<Inner, String> {
        // Safety: every call below follows the standard minimal Win32+WGL
        // "register a class, create a window, pick+set a pixel format,
        // create+make-current a GL context" recipe; every fallible step is
        // checked and every resource created before a later failure is torn
        // down before returning `Err`, so no partial window/DC/context
        // leaks.
        unsafe {
            let h_instance = GetModuleHandleW(ptr::null());
            let class_name = to_wide("FableWindowClass");
            let cursor = LoadCursorW(ptr::null_mut(), IDC_ARROW);

            let wc = WndClassExW {
                cb_size: std::mem::size_of::<WndClassExW>() as u32,
                style: 0,
                lpfn_wnd_proc: Some(wndproc),
                cb_cls_extra: 0,
                cb_wnd_extra: 0,
                h_instance,
                h_icon: ptr::null_mut(),
                h_cursor: cursor,
                h_background: ptr::null_mut(),
                lpsz_menu_name: ptr::null(),
                lpsz_class_name: class_name.as_ptr(),
                h_icon_sm: ptr::null_mut(),
            };
            // Registering the same class name twice (e.g. a second `Window`
            // in the same process) fails with `ERROR_CLASS_ALREADY_EXISTS`;
            // that's fine — the class is already usable — so its return
            // value is deliberately not checked, matching `x11.rs`'s general
            // "only check the calls whose failure actually blocks progress"
            // discipline.
            RegisterClassExW(&wc);

            let title_w = to_wide(title);
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                title_w.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                w,
                h,
                ptr::null_mut(),
                ptr::null_mut(),
                h_instance,
                ptr::null_mut(),
            );
            if hwnd.is_null() {
                return Err("window.create: CreateWindowExW failed".to_string());
            }

            let hdc = GetDC(hwnd);
            if hdc.is_null() {
                DestroyWindow(hwnd);
                return Err("window.create: GetDC failed".to_string());
            }

            let mut pfd: PixelFormatDescriptor = std::mem::zeroed();
            pfd.n_size = std::mem::size_of::<PixelFormatDescriptor>() as u16;
            pfd.n_version = 1;
            pfd.dw_flags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER;
            pfd.i_pixel_type = PFD_TYPE_RGBA;
            pfd.c_color_bits = 32;
            pfd.c_depth_bits = 24;
            pfd.i_layer_type = PFD_MAIN_PLANE;

            let format = ChoosePixelFormat(hdc, &pfd);
            if format == 0 {
                ReleaseDC(hwnd, hdc);
                DestroyWindow(hwnd);
                return Err("window.create: ChoosePixelFormat found no matching pixel format"
                    .to_string());
            }
            if SetPixelFormat(hdc, format, &pfd) == 0 {
                ReleaseDC(hwnd, hdc);
                DestroyWindow(hwnd);
                return Err("window.create: SetPixelFormat failed".to_string());
            }

            let hglrc = wglCreateContext(hdc);
            if hglrc.is_null() {
                ReleaseDC(hwnd, hdc);
                DestroyWindow(hwnd);
                return Err("window.create: wglCreateContext failed".to_string());
            }
            if wglMakeCurrent(hdc, hglrc) == 0 {
                wglDeleteContext(hglrc);
                ReleaseDC(hwnd, hdc);
                DestroyWindow(hwnd);
                return Err("window.create: wglMakeCurrent failed".to_string());
            }

            let state = Box::into_raw(Box::new(SharedState {
                pressed: HashSet::new(),
                mouse: (0.0, 0.0),
                width: w,
                height: h,
                should_close: false,
            }));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);

            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);

            Ok(Inner {
                hwnd,
                hdc,
                hglrc,
                state,
                mouse: (0.0, 0.0),
                width: w,
                height: h,
                should_close: false,
            })
        }
    }

    pub fn poll(&mut self) {
        // Safety: `PeekMessageW`/`TranslateMessage`/`DispatchMessageW` are
        // the standard non-blocking Win32 message-pump triad; `msg` is
        // zero-initialized before `PeekMessageW` fills it in.
        unsafe {
            let mut msg: Msg = std::mem::zeroed();
            while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // `wndproc` (running via `DispatchMessageW` above) mutated the
            // shared state through `GWLP_USERDATA`; mirror it onto `self`'s
            // public fields, exactly like `x11.rs`'s `poll` updates `self`
            // directly from the events it just pumped.
            let state = &*self.state;
            self.mouse = state.mouse;
            self.width = state.width;
            self.height = state.height;
            self.should_close = state.should_close;
        }
    }

    pub fn key_down(&self, name: &str) -> bool {
        let Some(vk) = vk_from_name(name) else {
            return false;
        };
        // Safety: `self.state` is a live `*mut SharedState` for the whole
        // lifetime of `self` (freed only in `teardown`, which consumes
        // `self`).
        let pressed = unsafe { &(*self.state).pressed };
        pressed.contains(&vk)
    }

    pub fn clear(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // Safety: makes this window's context current before issuing GL
        // calls — necessary if another `Window` made itself current since
        // this one was created (WGL contexts are current per-thread, not
        // per-window, exactly like GLX). `wglMakeCurrent` can fail (e.g. a
        // display-driver reset or an RDP disconnect/reconnect invalidates
        // existing WGL contexts) — skip the GL calls rather than issue them
        // with no context bound, which the GL spec leaves undefined.
        unsafe {
            if wglMakeCurrent(self.hdc, self.hglrc) != 0 {
                glClearColor(r, g, b, a);
                glClear(GL_COLOR_BUFFER_BIT);
            }
        }
    }

    pub fn swap_buffers(&mut self) {
        // Safety: same current-context caveat as `clear`.
        unsafe {
            if wglMakeCurrent(self.hdc, self.hglrc) != 0 {
                SwapBuffers(self.hdc);
            }
        }
    }

    /// Idempotent teardown, called by both `WindowHandle::close` and its
    /// `Drop` (see the module docs on `src/window/mod.rs`). Order: release
    /// current (only if *this* context is the one bound on this thread —
    /// blindly releasing would break a second, still-live `Window`),
    /// destroy the GL context, release the device context, destroy the
    /// window, free the boxed shared state.
    pub fn teardown(self) {
        // Safety: every handle here was produced by the matching Win32/WGL
        // create call in `Inner::create` and is torn down in the reverse
        // order it was created, exactly once (this method consumes `self`).
        // `self.state` was produced by `Box::into_raw` in `Inner::create`
        // and is reclaimed here via `Box::from_raw`, exactly once (again,
        // `self` is consumed), matching `Inner::create`'s one-time leak.
        unsafe {
            if wglGetCurrentContext() == self.hglrc {
                wglMakeCurrent(ptr::null_mut(), ptr::null_mut());
            }
            wglDeleteContext(self.hglrc);
            ReleaseDC(self.hwnd, self.hdc);
            DestroyWindow(self.hwnd);
            drop(Box::from_raw(self.state));
        }
    }
}

/// Map a `key_down` name to a Win32 virtual-key code. Single ASCII
/// letters/digits use the VK codes' documented equivalence to uppercase
/// ASCII (`'A'..'Z' = 0x41..0x5A`, `'0'..'9' = 0x30..0x39`); everything else
/// is a small hand-written table of the named keys worth covering — no
/// `XStringToKeysym`-style name→code library call exists on this platform,
/// so (unlike `x11.rs`) this is not exhaustive, matching the task's stated
/// scope.
fn vk_from_name(name: &str) -> Option<u32> {
    if name.len() == 1 {
        let c = name.chars().next().unwrap();
        if c.is_ascii_alphabetic() {
            return Some(c.to_ascii_uppercase() as u32);
        }
        if c.is_ascii_digit() {
            return Some(c as u32);
        }
    }
    Some(match name {
        "escape" => 0x1B,
        "space" => 0x20,
        "return" | "enter" => 0x0D,
        "tab" => 0x09,
        "backspace" => 0x08,
        "shift" => 0x10,
        "control" | "ctrl" => 0x11,
        "alt" => 0x12,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "f1" => 0x70,
        "f2" => 0x71,
        "f3" => 0x72,
        "f4" => 0x73,
        "f5" => 0x74,
        "f6" => 0x75,
        "f7" => 0x76,
        "f8" => 0x77,
        "f9" => 0x78,
        "f10" => 0x79,
        "f11" => 0x7A,
        "f12" => 0x7B,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end smoke test: create a window, clear it, swap buffers, pump
    /// events, confirm it isn't asking to close, then tear it down. Skips
    /// gracefully (doesn't panic the suite) if window creation fails for any
    /// environment-specific reason (e.g. no display session), matching
    /// `x11.rs`'s test's graceful-skip style. On `windows-latest` in CI —
    /// a real Windows machine, unlike this module's own author's Linux dev
    /// environment — this exercises the whole pipe for real.
    #[test]
    fn create_clear_swap_poll_close() {
        let inner = match Inner::create("fable window test", 320, 240) {
            Ok(inner) => inner,
            Err(e) => {
                eprintln!("skipping: {e}");
                return;
            }
        };
        let mut inner = inner;
        assert_eq!(inner.width, 320);
        assert_eq!(inner.height, 240);
        inner.clear(0.1, 0.2, 0.3, 1.0);
        inner.swap_buffers();
        inner.poll();
        assert!(!inner.should_close);
        inner.teardown();
    }
}
