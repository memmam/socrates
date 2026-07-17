//! Win32 window machinery shared by both rendering backends — the class
//! registration, `CreateWindowExW`, the `PeekMessageW` pump, key-name
//! mapping, and teardown that a window needs whether OpenGL/WGL
//! ([`super::gl`]) or Vulkan ([`super::vulkan`]) renders into it. The exact
//! structural twin of `x11/shared.rs` (and `macos/shared.rs`): each backend
//! composes a [`Win32WindowState`] rather than duplicating this.
//!
//! Struct layouts and function prototypes below are drawn from the frozen
//! Win32 ABI (`winuser.h`/`windef.h`, unrevised since Windows NT 3.1/95);
//! see the sourcing note in `gl.rs`'s module docs.
//!
//! **The WNDPROC / `GWLP_USERDATA` pattern.** Unlike Xlib (poll-based, no
//! callback), Win32 delivers window messages through a C callback
//! (`WNDPROC`) that carries no closure capture. The standard idiom — used
//! here — is to heap-allocate the mutable event state in a `Box`, stash the
//! raw pointer in the window's `GWLP_USERDATA` slot right after
//! `CreateWindowExW` returns a valid `HWND`, and have `wndproc` recover it
//! (null-checked: it's `0` for the handful of messages that arrive before
//! that `SetWindowLongPtrW` call, e.g. `WM_NCCREATE`/`WM_CREATE`) to mutate
//! through the raw pointer. [`Win32WindowState::teardown`] frees that box,
//! mirroring `x11/shared.rs`'s single-owner teardown discipline.

// This module's type names (`HWND`, `LPCWSTR`, `WNDPROC`, ...) match Win32's
// own naming exactly, on purpose — same call `x11/shared.rs` makes for
// `XID`.
#![allow(clippy::upper_case_acronyms)]
// In a vulkan-only build the whole module is (for now) consumer-less:
// `vulkan.rs` is Phase-0 scaffolding whose `create` errs before ever making
// a window. The WSI phase composes `Win32WindowState` from `vulkan.rs` and
// this allowance narrows away — the same arc `x11/shared.rs` followed.
#![cfg_attr(not(feature = "gl"), allow(dead_code))]

use std::collections::HashSet;
use std::ffi::c_void;
use std::ptr;

// ---------------------------------------------------------------------------
// Win32 types (winuser.h / windef.h) — shared by both backends. `HDC` lives
// here (GetDC/ReleaseDC are plain windowing calls) even though only the GL
// half uses one; `HGLRC` is WGL-specific and stays in `gl.rs`.
// ---------------------------------------------------------------------------

pub(super) type HWND = *mut c_void;
pub(super) type HDC = *mut c_void;
pub(super) type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HICON = *mut c_void;
type HCURSOR = *mut c_void;
type HBRUSH = *mut c_void;
type HMODULE = *mut c_void;
pub(super) type LPCWSTR = *const u16;
type LPVOID = *mut c_void;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, usize, isize) -> isize>;

/// `windef.h`.
#[repr(C)]
#[derive(Clone, Copy)]
struct POINT {
    x: i32,
    y: i32,
}

/// `winuser.h`. Field order confirmed via web search (see `gl.rs`'s module
/// docs for the sourcing caveat).
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

// winuser.h constants.
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
    // GetDC/ReleaseDC are `pub(super)`: the GL half owns a device context
    // for its pixel format + SwapBuffers (a Vulkan window needs none).
    pub(super) fn GetDC(hwnd: HWND) -> HDC;
    pub(super) fn ReleaseDC(hwnd: HWND, hdc: HDC) -> i32;
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
    pub(super) fn GetModuleHandleW(lp_module_name: LPCWSTR) -> HMODULE;
    fn SetWindowLongPtrW(hwnd: HWND, n_index: i32, dw_new_long: isize) -> isize;
    fn GetWindowLongPtrW(hwnd: HWND, n_index: i32) -> isize;
}

/// UTF-16-encode + NUL-terminate for the `W` API family.
pub(super) fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The boxed mutable event state `wndproc` mutates through
/// `GWLP_USERDATA` — see the module docs.
struct SharedState {
    pressed: HashSet<u32>,
    mouse: (f64, f64),
    width: i32,
    height: i32,
    should_close: bool,
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w_param: usize, l_param: isize) -> isize {
    // Safety: `GWLP_USERDATA` holds either `0` (messages that can arrive
    // before `create_window` installs it, e.g. `WM_NCCREATE`/`WM_CREATE`) or
    // a valid `*mut SharedState` produced by `Box::into_raw` in
    // `create_window` and freed only in `teardown`, so it is always either
    // null or live for the window's whole lifetime.
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

/// The Win32-generic half of a live window, composed by each backend's
/// `Inner` (the `x11/shared.rs::X11WindowState` pattern): the `HWND`, the
/// boxed `wndproc` state, and the event snapshot `poll` copies out of it.
pub(super) struct Win32WindowState {
    pub(super) hwnd: HWND,
    state: *mut SharedState,
    pub(super) mouse: (f64, f64),
    pub(super) width: i32,
    pub(super) height: i32,
    pub(super) should_close: bool,
}

impl Win32WindowState {
    /// Register the window class (idempotently), create the window, and
    /// install the boxed event state — everything up to but not including
    /// showing it: callers finish their rendering setup first and then call
    /// [`show`](Self::show), preserving the original single-backend
    /// creation order (window → GL context → show).
    ///
    /// # Safety
    /// Plain Win32 calls; every fallible step is checked, and the one
    /// resource pair created here (window + state box) is torn down by
    /// [`teardown`](Self::teardown), which the caller owns from `Ok` on
    /// (including its own later failure paths).
    pub(super) unsafe fn create_window(
        entry_point: &str,
        title: &str,
        w: i32,
        h: i32,
    ) -> Result<Win32WindowState, String> {
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
        // value is deliberately not checked, matching `x11/shared.rs`'s
        // general "only check the calls whose failure actually blocks
        // progress" discipline.
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
            return Err(format!("{entry_point}: CreateWindowExW failed"));
        }

        let state = Box::into_raw(Box::new(SharedState {
            pressed: HashSet::new(),
            mouse: (0.0, 0.0),
            width: w,
            height: h,
            should_close: false,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);

        Ok(Win32WindowState {
            hwnd,
            state,
            mouse: (0.0, 0.0),
            width: w,
            height: h,
            should_close: false,
        })
    }

    /// Show + first-paint the window — each backend calls this after its
    /// rendering setup succeeds (see [`create_window`](Self::create_window)).
    pub(super) fn show(&self) {
        // Safety: `self.hwnd` is live until `teardown`.
        unsafe {
            ShowWindow(self.hwnd, SW_SHOW);
            UpdateWindow(self.hwnd);
        }
    }

    pub(super) fn poll(&mut self) {
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
            // fields, exactly like `x11/shared.rs`'s `poll` updates itself
            // directly from the events it just pumped.
            let state = &*self.state;
            self.mouse = state.mouse;
            self.width = state.width;
            self.height = state.height;
            self.should_close = state.should_close;
        }
    }

    pub(super) fn key_down(&self, name: &str) -> bool {
        let Some(vk) = vk_from_name(name) else {
            return false;
        };
        // Safety: `self.state` is a live `*mut SharedState` for the whole
        // lifetime of `self` (freed only in `teardown`, which consumes
        // `self`).
        let pressed = unsafe { &(*self.state).pressed };
        pressed.contains(&vk)
    }

    /// Destroy the window and reclaim the boxed event state, exactly once
    /// (consumes `self`). Backends release their own rendering resources
    /// (GL context, device context, ...) *before* calling this, preserving
    /// the original reverse-creation teardown order.
    pub(super) fn teardown(self) {
        // Safety: `self.hwnd` was produced by `CreateWindowExW` and
        // `self.state` by `Box::into_raw`, both in `create_window`;
        // consuming `self` makes each reclaimed exactly once. The state box
        // is dropped after `DestroyWindow` so `wndproc` can still read it
        // for the messages destruction itself dispatches (`WM_DESTROY`).
        unsafe {
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
/// so (unlike `x11/shared.rs`) this is not exhaustive, matching the task's
/// stated scope.
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
