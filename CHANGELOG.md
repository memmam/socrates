# Changelog

Each release was shipped as one reviewed pull request. Golden spec tests pin
every feature listed here; `docs/SPEC.md` marks each with the version that
introduced it, and `CLAUDE.md` keeps the release ledger.

## Unreleased (v0.9) — the minification pass

The maximally performant set of minimal idioms that covers all
functionality, judged per-architecture. In progress:

- **The four-arch Bench A/B gate** (`bench/ab.py` + the Bench A/B
  workflow): every interpreter/idiom change is judged by interleaved A/B
  on one runner per tier-1 architecture; CLAUDE.md's universality
  principle is the acceptance rule. macOS is judged by multi-sample
  majority (`bench/RESULTS.md` documents the per-job modulation that
  makes single macOS runs unreliable below ~6%).
- **The compact dispatch loop with per-target binding**: `run()` keeps
  only compact, frequent arms inline; bulky or rare op bodies outline
  behind `#[inline(never)]` — killing the codegen lottery that made
  ±5–14% phantom swings out of unrelated edits — except on
  aarch64-linux, where a `build.rs`-emitted `monolithic_dispatch` cfg
  folds them back in (the compact loop measured a reproducible
  enum_match cost there; per CLAUDE.md, an irreconcilable per-target
  disagreement binds each target to its measured-fastest form instead
  of accepting a tradeoff). Broad wins elsewhere: up to −27% on
  Apple-Silicon micro benches, −3..−8% across x86_64 Linux/Windows.
- **`fft.magnitude` moved to `std.fft`** (pure Fable over the `fft`
  primitives; wrapper-shaped natives live in std). `import std.fft;`
  keeps the `fft.` spellings working.
- **The demo adoption gap, closed**: the v0.8 features added for
  specific demos are now used by them — bloom's `mul32` is
  `wrapping_mul` + mask, checkers' `lshr` is `ushr`, spreadsheet and
  mdsite join through `push_joined`, swarm builds its messages with the
  `std.json` constructors, regex reads bitmap words in one
  `read_u64le`, and the Int-key comparator sites use `max_by_key`.
  Byte-identical goldens; `demos/NOTES.md` records the lesson
  (adoption is part of shipping a queue item).
- **math namespace minified**: `math.sqrt/floor/ceil/round/abs/abs_int/
  min/max/min_float/max_float` dropped — verbatim duplicates of the
  Int/Float methods, which are the primitives (`x.sqrt()`, `a.min(b)`,
  ...). `Float.min`/`Float.max` added to complete the method set. `math`
  keeps what only it provides: trig, logs, `exp`, `pow`, `fmod`, the
  PRNG, and the constants.
- **`std.json` serializer fast path**: `escape()` returns clean strings
  (no `\` `"` `\n` `\t`) as-is instead of running four always-allocating
  `String.replace` passes — the common case goes four allocations to
  zero per string and per object key. Byte-identical output;
  bench_json −6.9% in the local interleaved A/B (confirmed on the
  four-arch matrix: −4.5..−7.5% across all tier-1 targets).
- **Consistency pass — SPEC↔implementation reconciliation**:
  `String.parse_hex()` now rejects a leading `+` (`"+ff"` → `None`),
  closing the one behavioral gap against SPEC's "no sign" rule; LSP
  namespace completion gained the six v0.8 members it was missing
  (`worker.try_recv`, `gpu.run_spirv`/`gpu.backend`,
  `window.create_metal`/`window.create_vulkan`,
  `gfx.compile_program_spirv`), and the unit test now asserts the
  completion lists and the resolver agree in both directions; stale
  wording fixed across SPEC (§ 7.1 module count, § 8.4d Windows
  Vulkan, fmod semantics, cross-references) and the gpu/window doc
  comments (SPIR-V's two consumers, the five-backend precedence).

## v0.8.0 — native graphics and compute; the demo round's feature queue

One release, two workstreams. First, the standing directive from
`CLAUDE.md`'s roadmap: replace the one quarantined dependency (wgpu) with
native raw-FFI backends for every graphics and compute API worth having,
built over a maximally-performant, minimal-duplication shared core —
three native windowing/draw-call backends, five native compute backends,
and, the headline, **every build of Fable is now zero-dependency**
(`Cargo.toml` has no `[dependencies]` section at all; CI asserts a
one-line `cargo tree` for the default build and for every feature set).
Second, the feature-request queue the v0.7 demo round left behind, worked
through directly (the second half of this section). 311 spec tests;
eighteen demos (`glcube` joins the zoo) with every golden byte-identical
throughout.

- **`std.glm`** — vector/matrix/quaternion math named and shaped after
  GLM (`vec3`, `perspective`, `look_at`, `proj.mul(view).mul(model)`),
  pure Fable; plus `Bytes` f32 accessors (`push_f32le`/`be`,
  `read_f32le`/`be`) for vertex data.
- **The `window` namespace** — real OS windows over raw FFI:
  `window.create` is OpenGL everywhere (X11/GLX on Linux, Win32/WGL on
  Windows, Cocoa/CGL on Apple Silicon macOS — `dlopen`ed GL, zero
  dependencies, behind the `gl` feature), with events, `key_down`,
  `mouse`, clear/present, and idempotent teardown.
- **The `gfx` namespace** — a GL-shaped draw-call surface (programs,
  buffers, vertex arrays, uniforms resolved by name, textures, draws,
  `read_pixels`) that behaves identically on every backend.
- **The Metal backend** (`--features metal`, Apple Silicon) — additive
  alongside GL, never a replacement: `window.create_metal` plus the full
  `gfx` surface in raw `objc_msgSend` FFI, MSL shaders, reflection-based
  uniforms. On macOS the interpreter now runs on the main thread (Cocoa's
  requirement), with the worker/main split handled at startup.
- **The Vulkan backend** (`--features vulkan`) — `window.create_vulkan`
  on Linux/X11 **and** Windows (`VK_KHR_xlib_surface` /
  `VK_KHR_win32_surface`), with everything past the surface — device
  pick, swapchain, offscreen back buffer, and the whole `gfx` surface —
  in one shared platform-neutral core (`window/vulkan.rs`), so the two
  platforms are behaviorally identical by construction. SPIR-V shaders
  via `gfx.compile_program_spirv` with in-house SPIR-V reflection; CI
  proves presentation and draws with real pixels on Mesa's lavapipe.
- **Three backends, one picture**: `demos/glcube` renders the same
  spinning cube with golden frame pins **byte-identical** across OpenGL,
  Metal, and Vulkan — the same Fable program, the same pixels, three
  graphics APIs.
- **Five native compute backends** — `gpu.run` / `gpu.run_spirv` over
  Metal (MSL source), Vulkan (SPIR-V, GLCompute profile), OpenCL
  (SPIR-V, Kernel profile via `clCreateProgramWithIL` — CI-proven on
  Intel's CPU runtime), CUDA (PTX text JIT'ed by the NVIDIA driver; no
  toolkit), and Direct3D 12 (HLSL compiled at dispatch by the OS's own
  `d3dcompiler_47.dll`; WARP guarantees a device, so CI hard-asserts
  real dispatched bytes). `gpu.backend()` names the live one; precedence
  is metal > vulkan > d3d12 > cuda > opencl; the two SPIR-V compute
  profiles are documented in SPEC § 7.2.
- **wgpu and pollster deleted** — v0.7's quarantined `gpu` feature and
  its WGSL dialect are gone, replaced by the native set; `Cargo.lock`
  went from 1212 lines to 7.
- **Shared cores, extracted at the second consumer** (the discipline,
  applied three times): `objc.rs`/`mtl.rs` (Objective-C dispatch + Metal,
  shared by windowing and compute), `vk.rs` (the Vulkan loader and 1.0
  primitives, shared by compute and windowing), and `window/vulkan.rs`
  (the entire Vulkan WSI + draw-call machinery, shared by the Linux and
  Windows shims — a net −1212 lines, and the lavapipe pixel asserts
  prove the exact code Windows runs).
- **The book** grew chapter 8 sections for `std.glm` and `window`/`gfx`
  (executable, like every snippet), and chapter 9's `gpu` section covers
  all five backends.

**The feature queue.** The v0.7 demo round (seventeen writers, seventeen
adversarial verifiers) left a deduplicated feature queue, ranked by how
many independent demos hit each wall (`demos/NOTES.md` § "The v0.7
round"); this release works through it directly:

- **`if let` / `while let`** (×3: dungeon, mdsite, parmandel): test a
  single pattern without a full `match`. Both are pure parser sugar,
  desugared fully at parse time — `if let PAT = E { T } else { F }` is
  exactly `match E { PAT -> T, _ -> F }`; `while let PAT = E { B }` is
  exactly `while true { match E { PAT -> B, _ -> break } }`, the drain-loop
  idiom STYLE.md already documented. The checker and compiler need no
  special cases; an irrefutable user pattern making the synthetic fallback
  arm unreachable is silently fine, not a warning — the user never wrote
  that arm.
- **Bitwise compound assignment** (×3: reversi, sudoku, wfc): `&= |= ^=
  <<= >>=`, matching the arithmetic set. Int-only, never dispatches,
  exactly like the plain bitwise operators.
- **Hex bit patterns ≥ 2⁶³, `String.parse_hex`** (×3: png, bloom, reversi):
  hex/binary literals now parse as the raw 64-bit pattern, so
  `0x8080808080808080` and `0x8000000000000000` (`Int`'s minimum) are both
  writable; `parse_hex()` is `to_hex()`'s inverse.
- **`Bytes` 64-bit accessors** (part of the ×5 "Bytes readers" ask):
  `push_u64le`/`push_u64be`/`read_u64le`/`read_u64be` — no range check
  needed at 64 bits, since `Int` already *is* the two's-complement value.
- **Builder ergonomics** (×3: spreadsheet, mdsite, plot): `is_empty()`,
  `push_joined(sep, s)` (pushes `sep` first unless this is the builder's
  first piece — the "gate on `len() > 0`" idiom, wrapped).
- **`fft.magnitude(re, im) -> List[Float]`** (×2: spectra, plot): every
  `rfft` consumer wrote this zip/hypot line by hand.
- Singles: `worker.try_recv()` (non-blocking `recv`,
  `Option[Option[String]]` — not-ready / hung-up / message — for a parent
  polling several workers without blocking on one); `std.lists`
  `min_by_key`/`max_by_key` (an `Int`-valued key extractor, alongside the
  comparator-based `min_by`/`max_by`); `fable test --bless` (rewrites
  mismatched `//? expect:` lines in place when the actual/expected line
  count already agrees — a count change still fails normally, since which
  new line pairs with which directive is then a human's call); `std.lazy`
  (`Lazy[T]`: deferred, memoized computation — `of(thunk)`, `.get()`,
  `.is_forced()` — the deferred half of the module-level-`let`-builds-once
  idiom); `Range.all`/`any` (short-circuiting, matching `List`'s, where
  before only `.to_list().any(..)` reached it); `Int.wrapping_add`/
  `wrapping_sub`/`wrapping_mul` (64-bit wrap for hash finalizers; a 32-bit
  wrap is `a.wrapping_mul(b) & 0xFFFFFFFF` — one primitive, not a second
  32-bit-specific intrinsic); ergonomic `std.json` construction
  (`json.obj`/`arr`/`jstr`/`num`/`int`/`bool`/`null`, named for what they
  build — `jstr`, not `str`, since this module's own code calls the
  builtin `str()` and a same-named local function would shadow it).
- **Declined:** a counting-map helper (checkers) — one demo, and
  `m.insert(k, m.get(k).unwrap_or(0) + 1)` is a single line; `std` grows
  reluctantly (`demos/NOTES.md`).
- Four items in the original queue turned out to already be shipped —
  `count_ones`/`leading_zeros`/`trailing_zeros`/`ushr`/`rotate_left`/
  `rotate_right`/`to_hex` and the Bytes BE pushers/readers/bulk-append all
  landed within v0.7's own efficiency pass; the queue predated that and
  was never updated to match.

## v0.7.0 — the infrastructure release

- `fable build <dir|file>` — pack a program into one self-contained
  executable. Every file the program touches (modules, data files, the
  `.fable` files it hands `worker.spawn`) is stapled onto a copy of the
  interpreter as a dependency-free appended payload; the binary reads its
  own 16-byte trailer at startup, unpacks into a scratch directory, and
  runs — so its output is byte-identical to `fable <path>` from source.
  Stapling is target-independent (`--launcher` supplies cross-compiled
  interpreter bytes), so one host assembles binaries for every target. The
  release ships the whole **demo zoo**: all seventeen demos cross-built for
  `x86_64`/`aarch64` Linux and Windows and Apple-Silicon macOS, as
  `fable-demozoo-<version>-<target>.tar.gz`. On macOS — where a payload
  appended past the Mach-O `__LINKEDIT` can't be code-signed — the payload is
  linked in as a `__DATA,__fablezoo` section instead (`ld -sectcreate`; read
  back by parsing the running image) and ad-hoc signed; Developer ID signing +
  notarization are wired to switch on once the signing secrets are configured.
- The efficiency pass: a measured, benchmark-gated optimization sweep
  (`bench/` is the yardstick; every change was interleaved-A/B'd, and
  negative results are recorded in the audit trail). Interpreter: frame-
  hot state hoisted into dispatch-loop locals, write-in-place binop and
  native-call stack traffic, allocation-free `for` over Int ranges,
  scalar fast paths for structural hashing, interned single-char ASCII
  strings, allocation-free GC mark phase with out-of-line mark bits,
  FMap single-entry buckets without SipHash. Natives: borrow-based
  string/list methods, pre-sized joins and HOF outputs. std: Builder
  re-backed by Bytes, json over UTF-8 bytes, deque/lists/set hot paths
  simplified. Net (interleaved vs pre-pass): checkers −15%, lisp −20%,
  string building −55%, map ops −37%, dispatch micros −14..19%,
  GC-stress suite time −67% on the heaviest demo. All 294 spec and 71
  demo goldens byte-identical throughout.
- Fast-idiom natives (the efficiency pass, batch 1): every bit-heavy
  demo in the v0.7 round hand-rolled the same primitives, so they are
  now intrinsics. `Int` grew `count_ones` / `leading_zeros` /
  `trailing_zeros` (the 0 case is 64 for both zero-counts, matching
  Rust), `ushr` (logical right shift, `>>`'s exact panic contract),
  `rotate_left` / `rotate_right` (count mod 64; never panic), and
  `to_hex` (lowercase minimal hex of the two's-complement pattern).
  `Bytes` grew bulk appends `push_bytes` (snapshot semantics —
  self-append works) / `push_str`, big-endian pushers `push_u16be` /
  `push_u32be` (same range checks as the LE trio), and multi-byte
  readers `read_u16le` / `read_i16le` / `read_u32le` / `read_u16be` /
  `read_u32be` (OOB panics match `get`). **The wrappers rule:** the
  demos' hand-rolled versions did not disappear — each became a
  minimal wrapper over the native with the same name and byte-identical
  observable behavior (`reversi/bits.fable` remains the documented
  reference; the hand-rolled bodies live in git history).
- The v0.7 demo round: six new demos (`synthwave`, `png`, `bloom`,
  `spectra`, `swarm`, `reversi`) built on the new infrastructure, all
  eleven existing demos modernized to it, seventeen writers plus
  seventeen adversarial verifiers. Best practices distilled into
  `demos/STYLE.md`; the papercut triage is `demos/NOTES.md` § "The
  v0.7 round".
- **Fixed (found by the round):** method calls and field access on
  module-qualified `pub let` members (`m.answer.to_float()`) no longer
  misresolve as enum paths; `worker.spawn` resolves relative files
  against the true entry script's directory even when the entry has
  imports; `fable fmt` formats every file argument (it silently took
  only the first); the formatter keeps interior comments of bracketed
  literals in place (they now pin the element-per-line layout — the
  official escape hatch for 2-D data tables); fitting `if/else-if`
  chains stay on one line and over-width ones break all branches
  consistently.

- `fable fmt` is now line-width-aware (v0.6 review note): constructs
  that fit in 100 columns keep their one-line layout; longer ones break
  the way an author would — call arguments one per line with a trailing
  comma, method chains before each `.` after the first, binary
  expressions before each operator, literals element-per-line, lambda
  bodies to a block — composing outermost-first. `--width N` overrides
  the limit; tokens are never split; comments and `//?` directives are
  preserved; formatting stays idempotent and behavior-preserving.
- `Bytes`: a packed byte-buffer primitive with checked accessors,
  little-endian multi-byte pushers (wire formats without bitwise
  operators), `slice`/`concat`/`to_list`, UTF-8 bridging to `String`,
  structural equality, and map-key support.
- `fs.read_bytes` / `fs.write_bytes` — binary file I/O, surfaced as a
  hard gap by the claudewave port (audio output needed WAV).
- `fft` namespace: native `fft.fft` / `fft.ifft` / `fft.rfft` over
  split-complex signals, any length ≥ 1 in O(n log n) (radix-2 for
  powers of two, Bluestein otherwise); numpy conventions,
  cross-checked against numpy in CI at 1e-9.
- Bitwise operators on Int: `&` `|` `^` `<<` `>>` (Rust's relative
  precedence; `>>` arithmetic; shift counts outside 0..=63 panic).
  The v0.6 "no bitwise operators" diagnostics retired.
- Workers: `worker.spawn(file, args)` runs a Fable program as an
  **isolate** — its own VM, heap, and GC on its own OS thread — joined
  to the parent by string channels (`send`/`recv`/`join` on the
  `Worker` handle; `worker.send`/`worker.recv`/`worker.is_worker`
  inside). Compile errors surface synchronously from `spawn`; a
  worker's panic is isolated and comes back as `Err` from `join`.
- `ports/`: the porting programme — `jsl` (JS/TSL layer, ICAA port,
  cross-validated to pixel equality) and `pyl` (Python/numpy layer,
  claudewave DSP core, in progress).
- **`gpu` namespace (experimental, feature-gated)**: `gpu.available()`,
  `gpu.adapter_info()`, and `gpu.run(wgsl, input, out_len, wx, wy, wz) ->
  Result[Bytes, String]` dispatch WGSL compute shaders over `Bytes` I/O
  (SPEC §7.2 documents the shader ABI). Implemented with wgpu behind the
  `gpu` cargo feature — the project's first-ever dependency, deliberately
  quarantined so the **default build stays zero-dependency** (CI asserts
  it). Without the feature the namespace still typechecks and degrades
  gracefully (`available()` is `false`, `run` returns `Err`). Demo:
  `docs/assets/gpu_double.fable`.
- `std` grows a collections layer: `std.set` (`Set[T]` over structural
  map keys; `insert`/`remove` report change, `union`/`intersect`/
  `difference` preserve insertion order), `std.deque` (two-stack
  `Deque[T]`, amortized O(1) at both ends), `std.lists` (`fill`,
  `sum`/`sum_float`, `min`/`max`/`min_float`/`max_float`,
  `min_by`/`max_by` — `sort_by` comparators, first winner on ties),
  and `strings.Builder` (`builder()`, `push`/`push_char`/`len`/
  `build`/`clear`) for O(n) string accumulation where a `+=` loop
  is O(n²).

## v0.6.0 — the field-test release

Ten demo programs (`demos/`) were written against v0.5 with orders to
report every papercut; ten independent authors hit the same dozen walls.
This release is those walls removed. Full triage: `demos/NOTES.md`.

- **Fixed:** `math.seed` collapsed adjacent seeds (42 and 43 produced
  identical streams); now SplitMix64-scrambled. Seeded streams are not
  stable across releases.
- **Fixed:** `fable test` treated `//?` anywhere in a line — including in
  strings and prose comments — as a directive; now it must begin the
  line's comment (full lexical model: strings, interpolation holes,
  nested block comments).
- `for` heads take irrefutable patterns: `for (i, x) in xs.enumerate()`.
- Bare `return`/`break`/`continue` as match-arm bodies; diverging arms
  unify with any type.
- A trailing `while true` with no `break` diverges; `os.exit` typechecks
  in any value position, like `panic`.
- `_` as a lambda parameter.
- Strings: `trim_start`, `trim_end`, `code_at`, `index_of_from`; free
  `char(code)`. Floats: `to_fixed(n)`. Math: `rand_int` (uniform via
  rejection sampling), `log10`, `fmod`.
- `FABLE_MAX_DEPTH` env var raises the 4,096-frame call-depth cap.
- Golden comparison ignores trailing whitespace; `fable test` rejects
  unknown flags.
- Targeted diagnostics: `{}` vs `{:}`, assignment as a match-arm body
  (including `+=`), `<<`/`>>`.
- Spec: struct-literal field shorthand documented (long-standing
  behavior), sorts documented stable, `math.log` documented natural.
- The v0.6 diff itself was adversarially reviewed before shipping; all
  confirmed findings fixed in-release (`demos/NOTES.md` § "The review
  round").

## v0.5.0 — closing the loop

- REPL imports: modules (including `std.*`) load, persist, and roll back
  cleanly across inputs.
- LSP completion: methods, fields, tuple indices, module members,
  namespaces, and top-level names — answered from the last good analysis,
  so it works mid-edit.
- The book became a test suite: every ```fable block in `book/` executes
  in CI, with fence tags for deliberate errors/panics.

## v0.4.0 — the toolchain release

- `fable test`: any `.fable` file with `//? expect/error/panic`
  directives is a golden test; the spec suite runs through the same code.
- Embedded standard library: `std.json`, `std.flags`, `std.path`,
  `std.strings`, `std.iter` (lazy iterators) — written in Fable, compiled
  into the binary.
- `fable lsp`: diagnostics, hover, go-to-definition over stdio; JSON-RPC
  hand-rolled.
- `try(f)`: catchable panics with full VM-stack restoration.
- GC pacing tuned (closure-churn benchmark 161ms → 38ms).

## v0.3.0 — the glue release

- `pub` visibility: module items private by default.
- Operator methods: `+ - * / %` and unary `-` dispatch to user methods;
  `==` stays structural.
- `FABLE_PATH` module search path.
- `fs.*` and `os.*` namespaces, Result-based and `?`-friendly.

## v0.2.0 — everything v0.1 declared out of scope

- `impl` blocks: methods on user structs and enums, generic impls.
- The `?` operator for `Option` and `Result`.
- Multi-file modules: `import a.b;` with diamond dedup and cycle
  detection.
- Tail-call optimization: frame reuse for calls in tail position.

## v0.1.0 — the language

Lexer, parser, unification-based inference with generics, Maranget
exhaustiveness checking, bytecode compiler, stack VM, mark-sweep GC,
REPL, formatter, disassembler, golden-test harness, spec, book, and
examples — zero dependencies.
