# glcube — a spinning cube over raw OpenGL

A unit cube, spinning about its vertical axis, rendered with the `gfx`
native draw-call namespace (SPEC § 7.4) on top of the `window` namespace
(§ 7.3), with the model/view/projection matrix built out of `std.glm`'s
`Mat4` (§ 7.1). Each of the cube's six faces is a solid color (front red,
back green, left blue, right yellow, top cyan, bottom magenta) — not for
looks, but so the golden test can identify *which face is on screen* from
a single pixel read-back.

## Why pixel spot-checks, not a framebuffer hash

CLAUDE.md's own convention: GL rendering (like seeded randomness) is
**stable only within a release, not across** — driver, GPU, and even
software-rasterizer versions can shift antialiasing and sub-pixel
rounding at triangle edges. Hashing the whole framebuffer would pin all of
that incidental detail. Instead, `main.fable` reads back single pixels
(`gfx.read_pixels(x, y, 1, 1)`) at coordinates chosen to sit deep inside
one face's solid interior, nowhere near an edge — so the exact byte
values (`0` or `255`, since every face color is pure primaries) hold
regardless of exactly how a given driver rasterizes the two triangles
underneath.

Two frames are checked: frame 0 (no rotation — the front face, red, faces
the camera dead-on) and the last frame (a quarter turn about Y — the left
face, blue, has rotated to face the camera dead-on instead). Both are
"straight down the view ray" angles chosen deliberately — see the comment
in `main.fable` for why a 45-degree frame would make a fragile pin (the
view ray would land exactly on the front/left edge).

## This demo needs a live GL context

Unlike every other demo in this directory, `main.fable`'s golden test only
passes when built with `--features gl` and run under a real X/GLX display
(a physical one, or Xvfb). Without either, `window.create` returns `Err`
and the program prints one tolerant line and exits 0 (the same convention
`docs/assets/gpu_double.fable` uses) — but that line doesn't match the
pinned rendered-pixel output, so the golden check itself only passes with
a working GL context. `cube.fable` and `spec.fable` need no GL context at
all (pure geometry and matrix math) and pass on every build.

## Run it

From the repo root:

```
cargo build --release --features gl
Xvfb :98 -screen 0 1024x768x24 &
DISPLAY=:98 ./target/release/fable demos/glcube/main.fable   # render
DISPLAY=:98 ./target/release/fable test demos/glcube          # golden tests
```

## The Metal twin: `main_metal.fable`

The same cube, through the same `gfx.*` calls, against a
`window.create_metal` window (macOS/Apple Silicon, `--features metal`).
`cube.fable` is reused completely unchanged; the only differences are the
create call and the shader source text (MSL instead of GLSL — the one
deliberate per-backend difference, per `win.backend_name()`'s design),
plus one line in the vertex shader remapping GL's `[-w, +w]` clip-space z
onto Metal's `[0, +w]` so `mvp_at`'s GL-convention projection works
verbatim. Its golden pins are **byte-identical** to `main.fable`'s — the
same two frames, the same pixel coordinates, the same expected values —
which is the point: it is the cross-backend pixel-parity proof, asserted
in CI by the `gl-macos-metal` job on real Apple Silicon hardware (the
Linux `gl` job's Xvfb can't run it, so that job pins `main.fable` and
lists its files explicitly).

## Files

| File          | What it is                                                         |
|---------------|---------------------------------------------------------------------|
| `cube.fable`  | the cube's 24-vertex/36-index geometry (one solid color per face) and `mvp_at`, the model\*view\*projection builder |
| `main.fable`  | opens the window, compiles the shader pair, spins the cube six frames, pixel-spot-checks two of them |
| `main_metal.fable` | the same scene on the Metal backend, pinned to identical pixels (see above) |
| `spec.fable`  | GL-free checks: vertex/index counts, and `mvp_at`'s screen-center projection at two rotation angles |

## Fable features on display

Native `gfx.*` draw calls (compile/link a GLSL 330 core pass-through
shader pair, a VBO + EBO + VAO with two interleaved vertex attributes,
a `mat4` uniform, indexed drawing, depth testing) and `window.make_current`
targeting them; `std.glm`'s `Mat4.mul`/`mul_vec4` composition
(`projection.mul(view.mul(model))`) and `rotation_y`/`look_at`/
`perspective`; `Bytes.push_f32le`/`push_u32le` building the interleaved
vertex buffer and the index buffer; tuple-destructuring loops building
each face's flattened vertex data; `Result`'s `?` operator threading
`window.create`/`gfx.compile_program`'s fallible setup through `run()`.
