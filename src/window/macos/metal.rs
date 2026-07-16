//! Metal backend for macOS, additive alongside `gl.rs` (OpenGL/CGL) — never
//! a replacement. A single compiled binary can hold either kind of window,
//! or both at once (`--features gl,metal`); see `super::Inner`'s two-variant
//! enum, which is the only place either backend's concrete type is named.
//!
//! **Phase 0 (this commit): scaffolding only.** [`Inner::create`] returns a
//! clean `Err` — no `MTLDevice`/`CAMetalLayer`/command-queue plumbing yet.
//! This exists so `window.create_metal`/`win.backend_name()` are wired
//! end-to-end and real CI (`gl-macos-metal`, macos-14) can confirm
//! `--features metal` and `--features gl,metal` both build/link/test clean
//! before any real Metal FFI lands. Device/queue/pipeline setup, real draw
//! calls, and the `gfx.*`-backing methods are follow-up commits (see
//! `/root/.claude/plans/greedy-swinging-volcano.md`'s phasing).

pub struct Inner;

impl Inner {
    pub fn create(_title: &str, _w: i32, _h: i32) -> Result<Inner, String> {
        Err("window.create_metal: Metal backend not yet implemented (device/queue/pipeline \
             plumbing lands in a follow-up commit)"
            .to_string())
    }
}
