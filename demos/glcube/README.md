# glcube — a spinning cube over raw OpenGL

A unit cube, spinning about its vertical axis, rendered with the `gfx`
native draw-call namespace (SPEC § 7.4) on top of the `window` namespace
(§ 7.3), with the model/view/projection matrix built out of `std.glm`'s
`Mat4` (§ 7.1). Each of the cube's six faces is a solid color (front red,
back green, left blue, right yellow, top cyan, bottom magenta) — not for
looks, but so the golden test can identify *which face is on screen* from
a single pixel read-back.

## Why pixel spot-checks, not a framebuffer hash

PROJECT.md's own convention: GL rendering (like seeded randomness) is
**stable only within a release, not across** — driver, GPU, and even
software-rasterizer versions can shift antialiasing and sub-pixel
rounding at triangle edges. Hashing the whole framebuffer would pin all of
that incidental detail. Instead, `main.soc` reads back single pixels
(`gfx.read_pixels(x, y, 1, 1)`) at coordinates chosen to sit deep inside
one face's solid interior, nowhere near an edge — so the exact byte
values (`0` or `255`, since every face color is pure primaries) hold
regardless of exactly how a given driver rasterizes the two triangles
underneath.

Two frames are checked: frame 0 (no rotation — the front face, red, faces
the camera dead-on) and the last frame (a quarter turn about Y — the left
face, blue, has rotated to face the camera dead-on instead). Both are
"straight down the view ray" angles chosen deliberately — see the comment
in `main.soc` for why a 45-degree frame would make a fragile pin (the
view ray would land exactly on the front/left edge).

## This demo needs a live GL context

Unlike every other demo in this directory, `main.soc`'s golden test only
passes when built with `--features gl` and run under a real X/GLX display
(a physical one, or Xvfb). Without either, `window.create` returns `Err`
and the program prints one tolerant line and exits 0 (the same convention
`docs/assets/gl_triangle.soc` uses) — but that line doesn't match the
pinned rendered-pixel output, so the golden check itself only passes with
a working GL context. `cube.soc` and `spec.soc` need no GL context at
all (pure geometry and matrix math) and pass on every build.

## Run it

From the repository root:

```
cargo build --release --features gl
Xvfb :98 -screen 0 1024x768x24 &
DISPLAY=:98 ./target/release/socrates demos/glcube/main.soc   # render
DISPLAY=:98 ./target/release/socrates test demos/glcube/main.soc demos/glcube/cube.soc demos/glcube/spec.soc  # golden tests
```

(Named files, not the bare directory: `main_metal.soc`/`main_vulkan.soc`
need `--features metal`/`--features vulkan` respectively — the same
scoping the `gl` CI job uses.)

## The Metal twin: `main_metal.soc`

The same cube, through the same `gfx.*` calls, against a
`window.create_metal` window (macOS/Apple Silicon, `--features metal`).
`cube.soc` is reused completely unchanged; the only differences are the
create call and the shader source text (MSL instead of GLSL — the one
deliberate per-backend difference, per `win.backend_name()`'s design),
plus one line in the vertex shader remapping GL's `[-w, +w]` clip-space z
onto Metal's `[0, +w]` so `mvp_at`'s GL-convention projection works
verbatim. Its golden pins are **byte-identical** to `main.soc`'s — the
same two frames, the same pixel coordinates, the same expected values —
which is the point: it is the cross-backend pixel-parity proof, asserted
in CI by the `gl-macos-metal` job on real Apple Silicon hardware (the
Linux `gl` job's Xvfb can't run it, so that job pins `main.soc` and
lists its files explicitly).

## The Vulkan twin: `main_vulkan.soc`

The same cube a third time, against a `window.create_vulkan` window
(Linux, `--features vulkan`). `cube.soc` is again reused unchanged; the
differences are the create call and the shader *input* — precompiled
SPIR-V binaries through `gfx.compile_program_spirv` (Vulkan has no
runtime GLSL compiler, and zero-dep forbids shipping one), hand-assembled
with the GLSL equivalents in the file's comments. The vertex module
carries the same clip-z remap line as the Metal twin's MSL, for the same
reason; Y needs no shader handling at all (the backend renders with a
negative-height viewport, so clip-space +Y is up as in GL). Its golden
pins are **byte-identical** to both `main.soc`'s and
`main_metal.soc`'s — the same Socrates program rendering the same pixels
on three graphics APIs — asserted in CI by the `vulkan` job under Xvfb +
Mesa's lavapipe (no GPU needed; this is the one glcube twin a plain
ubuntu runner can render).

## Files

| File          | What it is                                                         |
|---------------|---------------------------------------------------------------------|
| `cube.soc`  | the cube's 24-vertex/36-index geometry (one solid color per face) and `mvp_at`, the model\*view\*projection builder |
| `main.soc`  | opens the window, compiles the shader pair, spins the cube six frames, pixel-spot-checks two of them |
| `main_metal.soc` | the same scene on the Metal backend, pinned to identical pixels (see above) |
| `main_vulkan.soc` | the same scene on the Vulkan backend via SPIR-V shaders, pinned to identical pixels (see above) |
| `spec.soc`  | GL-free checks: vertex/index counts, and `mvp_at`'s screen-center projection at two rotation angles |

## Socrates features on display

Native `gfx.*` draw calls (compile/link a GLSL 330 core pass-through
shader pair, a VBO + EBO + VAO with two interleaved vertex attributes,
a `mat4` uniform, indexed drawing, depth testing) and `window.make_current`
targeting them; `std.glm`'s `Mat4.mul`/`mul_vec4` composition
(`projection.mul(view.mul(model))`) and `rotation_y`/`look_at`/
`perspective`; `Bytes.push_f32le`/`push_u32le` building the interleaved
vertex buffer and the index buffer; tuple-destructuring loops building
each face's flattened vertex data; `Result`'s `?` operator threading
`window.create`/`gfx.compile_program`'s fallible setup through `run()`.
