Fable is a statically-typed, garbage-collected programming language — ADTs,
exhaustive pattern matching, closures, generics, modules, a test runner, a
language server, a REPL, and an embedded standard library — implemented from
scratch in about 40,000 lines of Rust with **zero dependencies**. As of this
release that sentence has no asterisk: `Cargo.toml` has no `[dependencies]`
section at all, in the default build and in every feature set, and CI
asserts it (`cargo tree` is a single line everywhere).

v0.9 is the native graphics and compute release. The one quarantined
dependency v0.7 allowed itself (wgpu, behind a `gpu` feature) has been
replaced — and then some — by native raw-FFI backends for every graphics
and compute API worth having, built over shared cores extracted wherever a
second consumer appeared: three windowing/draw-call backends and five
compute backends, all `dlopen`/`objc_msgSend`/COM against the OS's own
libraries, nothing vendored, nothing downloaded.

Highlights of v0.9 over v0.8 (full list in
[`CHANGELOG.md`](https://github.com/memmam/fable/blob/main/CHANGELOG.md)):

- **The `window` and `gfx` namespaces** — real OS windows and a GL-shaped
  draw-call surface (programs, buffers, vertex arrays, uniforms by name,
  textures, draws, `read_pixels`). `window.create` is OpenGL on Linux/X11,
  Windows (WGL), and Apple Silicon macOS (CGL).
- **A Metal backend**, additive alongside GL on Apple Silicon —
  `window.create_metal`, MSL shaders, the full `gfx` surface.
- **A Vulkan backend** on Linux *and* Windows — `window.create_vulkan`,
  SPIR-V shaders (`gfx.compile_program_spirv`, with in-house SPIR-V
  reflection so `set_uniform_*` still resolves by name). Everything past
  the platform's surface is one shared backend, so the two platforms are
  behaviorally identical by construction — and CI proves presentation and
  draws with real pixels on Mesa's lavapipe.
- **One picture, three APIs**: the `glcube` demo renders the same spinning
  cube with golden frame pins byte-identical across OpenGL, Metal, and
  Vulkan.
- **`std.glm`** — vector/matrix/quaternion math named and shaped after GLM
  (`vec3`, `perspective`, `look_at`, `proj.mul(view).mul(model)`), in pure
  Fable.
- **Five native compute backends** behind `gpu.run`/`gpu.run_spirv`:
  Metal (MSL), Vulkan (SPIR-V), OpenCL (SPIR-V via
  `clCreateProgramWithIL`), CUDA (PTX, JIT'ed by the driver — no toolkit),
  and Direct3D 12 (HLSL, compiled at dispatch by the OS's own compiler;
  WARP guarantees a device on every Windows machine). `gpu.backend()`
  names the live one.
- **wgpu and pollster deleted** — `Cargo.lock` went from 1212 lines to 7.

Everything observable is pinned: 311 golden spec tests, 136 executable book
snippets, and 73 demo golden tests, the whole suite green under
`FABLE_GC_STRESS=1` — and the graphics backends are pinned with real
pixels, in CI, on plain runners (lavapipe for Vulkan, Xvfb for GL, macOS
runners for Metal, WARP for D3D12 compute).

## The demo zoo

Every one of the eighteen [`demos/`](https://github.com/memmam/fable/tree/main/demos)
— a Lisp, a spreadsheet, a backtracking regex engine, checkers with an
alpha-beta engine, a from-scratch PNG encoder, a chiptune renderer, a
parallel Mandelbrot, a spinning GL/Metal/Vulkan cube, and ten more — ships
as a **self-contained binary**: no `fable`, no source, one file you run.
They are attached as `fable-demozoo-v0.9.0-<target>.tar.gz` for five
desktop targets:

- `x86_64-linux`, `aarch64-linux`
- `x86_64-windows`, `aarch64-windows`
- `aarch64-macos` (Apple Silicon)

Unpack with `tar -xf` (built in on Windows 10+ too) and run any animal in the
zoo. On macOS the payload rides in a Mach-O section (appending it would break
code signing); the binaries are ad-hoc signed, so a downloaded copy needs
Gatekeeper cleared once — `xattr -d com.apple.quarantine ./<demo>` — until a
notarized build lands.

## Getting started

Unpack the attached interpreter for your platform (or `cargo build --release`
from source on anything Rust supports), then:

```sh
fable examples/mandelbrot.fable
fable demos/checkers/main.fable      # watch it play itself
fable test tests/spec demos          # run the whole golden suite
fable build demos/lisp -o lisp && ./lisp   # or make your own standalone binary
fable repl
```

Licensed under the Apache License 2.0 — see `LICENSE` and `NOTICE`.
