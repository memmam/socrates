// On macOS the interpreter runs on the process's real main thread — AppKit
// hard-requires NSWindow creation there (see src/main.rs's `main` and
// src/window/macos/shared.rs::is_main_thread) — so the deep-program stack
// every other platform gets from a spawned thread's `stack_size` is sized at
// link time here instead: 0x20000000 = 512 MiB, the same figure src/main.rs
// uses, a multiple of Apple Silicon's 16 KiB page size as ld64 requires.
//
// A build script rather than `.cargo/config.toml` `rustflags` deliberately:
// config-file rustflags are *replaced* by a `RUSTFLAGS` environment variable,
// and this repo's own CI sets one on macOS (`-Clink-arg=-Wl,-sectcreate,...`
// for section-embedded `socrates build` binaries — see ci.yml's
// macos-singlefile job and release.yml's demo zoo), which would silently
// drop the stack flag from exactly those binaries. `cargo:rustc-link-arg-*`
// composes with RUSTFLAGS instead, so both link args always apply.
//
// This adds no dependency (`cargo tree -e normal` stays one line) and emits
// nothing on any non-macOS target.
fn main() {
    // TARGET is the triple being built *for* (build scripts themselves run
    // on the host), so cross-compiled macOS binaries get the flag too.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target == "aarch64-apple-darwin" {
        println!("cargo:rustc-link-arg-bins=-Wl,-stack_size,0x20000000");
    }

    // Per-target implementation binding (bench/RESULTS.md, "The dispatch
    // restructure"): when one form of an implementation is measurably best
    // on some targets and an invariant loss on another, each target binds
    // its measured-fastest form through a cfg emitted here — one predicate,
    // one place to record which targets bind which form. First instance:
    // the VM dispatch loop's arm bodies outline behind #[inline(never)]
    // everywhere (the compact loop that killed the codegen lottery) except
    // aarch64-linux, which measured the monolithic loop faster (enum_match
    // +4.5% under outlining, reproduced across two layouts).
    if target.starts_with("aarch64") && target.contains("linux") {
        println!("cargo:rustc-cfg=monolithic_dispatch");
    }
    println!("cargo:rustc-check-cfg=cfg(monolithic_dispatch)");
    println!("cargo:rerun-if-changed=build.rs");
}
