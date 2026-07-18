# Ports — foreign code, translated into Socrates

This directory is a porting programme: bringing real code from other
ecosystems (JavaScript first, Python next) into Socrates through a shared
**translation layer**, so that porting is a mechanical, reviewable act
rather than a rewrite. Two long-term goals:

1. **Kitbashing** — pull useful pieces of existing tools and frameworks
   into Socrates projects without adopting their runtimes.
2. **Gradual factoring-out** — replace JS- and Python-based components of
   larger systems one module at a time, with each port validated against
   its original before it takes over.

## The layer: `jsl/`

`jsl` (JavaScript layer) holds Socrates modules that reproduce the host
vocabulary a ported file expects, so the port reads line-for-line like its
source. It grows by pull — each port adds only what it actually needed
(the same rule the language itself follows; see `demos/NOTES.md`).

Current modules (import with `SOCRATES_PATH=ports`, e.g. `import jsl.vec;`):

| Module | Provides | Grown by |
|--------|----------|----------|
| `jsl.vec` | GLSL/TSL-style `Vec2/3/4` with componentwise operators, `dot`, `norm`, swizzle helpers | icaa |
| `jsl.shade` | shader intrinsics: `clampf`, `mix`/`mix3`, `smoothstep`, `inverse_sqrt`, `luma`, exact three.js sRGB EOTF/OETF | icaa |
| `jsl.image` | a CPU texture: interleaved-RGBA `Image`, GPU-convention bilinear `sample` (clamp-to-edge, half-texel centers), PPM I/O | icaa |

### The mapping table

The core dialect translations, chosen once and reused by every port:

| JS / TSL | Socrates | Why |
|----------|-------|-----|
| `a.add(b)`, `a.sub(b)`, `a.mul(b)` (vec ∘ vec) | `a + b`, `a - b`, `a * b` | operator methods, componentwise |
| `a.mul(k)`, `a.add(k)` (vec ∘ scalar) | `a.scale(k)`, `a.adds(k)` | one signature per operator in Socrates |
| `x.oneMinus()` | `1.0 - x` | plain floats need no wrapper |
| `select(c, a, b)` | `if c { a } else { b }` | identical semantics for effect-free operands |
| `If(cond, () => return X)` | `if cond { return X; }` | shader early-out |
| `x.toVar()`, `x.assign(y)`, `x.addAssign(y)` | `let mut x = ...`, `x = y`, `x += y` | |
| build-time JS loops/arrays over nodes | runtime loops/`List`s, same iteration order | numerical parity |
| `c.rgb`, `vec4(v3, a)` | `c.rgb()`, `v3.to4(a)` | no swizzles in Socrates |
| `Math.abs(x)` (JS-side) vs `abs(node)` | both `x.abs()` / componentwise `abs_v` | |
| camelCase | snake_case, 1:1 (`pairCross` → `pair_cross`) | reviewable side-by-side |

**Numerical ground rules** (what makes cross-validation possible): all math
is f64 on both sides; ports keep the source's expression order and
associativity — no algebraic simplification; magic constants (epsilons,
sRGB curve coefficients, luma weights) are copied digit-for-digit.

### The validation pattern

Every port ships with its **receipts**:

1. A **plain-JS CPU reference** (`<port>/reference/*.mjs`, node, zero
   dependencies) — an independent transliteration of the same source.
2. A deterministic **input suite** generated in Socrates.
3. A pixel/value **diff harness** proving the Socrates port and the JS
   reference agree exactly (or to a documented, justified last-bit
   tolerance) across the whole suite.
4. Golden `socrates test` directives pinning the port's behavior in CI.

Two independent translations of one source that agree exactly are strong
evidence both are faithful; where they disagree, the divergence localizes
the bug in minutes via each implementation's `probe` mode.

## Ports

This table lists every port under `ports/` — a port that isn't here is
missing from the record, not from the tree; add the row in the same PR
that adds the port.

| Port | Source | Layer | Status |
|------|--------|-------|--------|
| [`icaa/`](icaa/) | [SkyeShark/icaa-antialiasing](https://github.com/SkyeShark/icaa-antialiasing) — ICAA, a novel single-frame post-process anti-aliasing (three.js TSL/WebGPU node), MIT | `jsl/` | see `icaa/README.md` |
| [`claudewave/`](claudewave/) | [SkyeShark/claudewave](https://github.com/SkyeShark/claudewave) — the DSP core (`lib/`) of a vaporwave/citypop remix toolkit: synth voices, drums, tape/vinyl treatments, ambience, an 18-band channel vocoder, MIT | `pyl/` | see `claudewave/README.md` |
