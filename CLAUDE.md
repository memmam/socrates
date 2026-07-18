# Socrates — project memory

Socrates is a statically-typed, garbage-collected programming language with
algebraic data types, exhaustive pattern matching, closures, and generics,
implemented from scratch in Rust with **zero dependencies** — every build.
This file is the working memory for the project: what Socrates is *for*, the
invariants that must never break, the engineering principles that decide
close calls, how to verify a change, where the detailed records live, and a
terse release ledger to draw on when writing changelogs and release posts.

## What Socrates is for

**The name** (recorded 2026-07-18, when the language was renamed from
Fable): trademark pre-emption and namespace collisions — an established
F#-to-JS compiler already holds "Fable". "Socrates" names the language's
substrate role; "Timaeus" was considered and is reserved for the eventual
top-of-stack agent; "Quine" was considered and rejected (an existing OSS
graph database holds it). The `.soc` extension nods at the
system-on-a-chip trajectory of the HDL roadmap. Git history preserves the
old name; `bench/ab.py` and the Bench A/B workflow carry the permanent
cross-name fallback that keeps pre-rename refs benchable.

Socrates is an **AI-native language**: its design mirrors the way current
frontier models reason, so an AI writes it fluently and uses it as a
recursive force multiplier — a substrate for building the tools that build
more tools. When AI-authorship fluency and human readability/ergonomics
pull in different directions, AI-fluency wins — this is not a language
optimized for a human encountering it cold. Naming that mirrors a
well-known API an AI already has deep fluency in (GLM's `vec3`/`perspective`/
`look_at`, GLSL/TSL shapes, POSIX/Win32 call shapes) beats naming chosen for
human intuition every time the two conflict. The intended trajectory, in
rough order:

- **Agent tooling** — MCP servers, client/harness code, glue — written in a
  language an AI can produce correctly on the first pass and golden-test
  instantly.
- **Hardware testing → an HDL pipeline.** Bit-exact integer semantics,
  `Bytes`, and bitwise intrinsics are load-bearing here; the near-term
  target is the user's 9-bit toy ISA (an AI-coprocessor design) — an
  assembler, emulator, and conformance battery in Socrates, then codegen.
- **Transpilation *into* Socrates** from Python and JavaScript and their
  frameworks (numpy/scipy already shimmed as `pyl`; Three.js-class and AIML
  frameworks next). The `ports/` programme (`jsl`, `pyl`) is the seed: run
  upstream unmodified against a Socrates-backed shim, cross-validate to
  numeric/pixel equality.
- **Transpilation *from* Socrates to raw Rust**, reaching for `unsafe`,
  pre-existing binary blobs, or external dependencies **only where
  necessary** — the same zero-dep-by-default discipline the interpreter
  holds itself to.

Keep this arc in mind when weighing features: the ones that serve
AI-authorship, bit-exact systems work, parallelism, and transpilation earn
their place fastest.

## Engineering principles (how to decide close calls)

- **Fastest, most parallelizable idiom wins.** When several ways express the
  same thing, prefer the one that is fastest *and* cleanest to parallelize,
  without losing capability. If picking it would drop functionality, add a
  new function or a thin wrapper — whichever is more performant — so nothing
  is lost. Err toward simplicity and performance.
- **Parallel compute is a first-class citizen**, not an afterthought:
  workers (OS-thread isolates), and the feature-gated GPU path, are core,
  and new hot paths should be designed to fan out cleanly.
- **The most performant version of an idiom becomes the primitive; older
  spellings become minimal wrappers over it** (the efficiency-pass rule —
  hand-rolled popcount/ushr/hex became one-line wrappers over natives).
  Every such change stays byte-identical in observable behavior.
- **Universality gates minification.** "Most performant" is judged across
  every tier-1 target (x86_64 + aarch64 Linux, x86_64 Windows, aarch64
  macOS — the release matrix), not on one box: simplifying an idiom down
  to pure primitives can run better on one architecture and worse on
  another (I-cache geometry, indirect-branch cost, and code layout all
  vote differently per arch — the dispatch-loop codegen lottery is the
  recorded instance). A simplification is accepted only if the
  interleaved A/B (`bench/ab.py`, fanned per-arch by the Bench A/B
  workflow) shows flat-or-better on every architecture; where
  architectures disagree, scope **up**, not down — the primitive keeps
  its place, and the minimal set is the minimal set of *universally*
  performant idioms. And when the disagreement is *irreconcilable* — an
  implementation form that is measurably best on some targets and a
  reproducible invariant loss on another — a tradeoff is never accepted:
  that is the signal to write the missing idiom that defers to the more
  performant implementation **per target**. Keep one source of truth for
  the behavior and bind each target to its measured-fastest form (a
  build.rs-emitted cfg names the binding; `monolithic_dispatch` is the
  first instance — vm.rs's dispatch-arm bodies outline into the compact
  loop everywhere except aarch64-linux, which measured the monolith
  faster and inlines them back).
- **This applies to whole backend implementations, not just algorithmic
  idioms** — but the trigger is the *platform* actually dropping the older
  path, not merely deprecating it. When a newer backend for the same
  capability is more optimal and the platform has genuinely retired the old
  one (eventually Vulkan/DirectX superseding GL, say), the newer backend
  supersedes rather than the two living alongside each other indefinitely —
  the older path is wrapped, thinned, or dropped as the better one lands.
  **Metal on macOS is the standing exception, not an instance of this
  rule**: it ships additive alongside OpenGL/CGL, not as a replacement.
  Apple marking OpenGL deprecated is not the same as Apple removing it —
  both backends stay until and unless macOS itself actually drops OpenGL
  support. The Socrates-facing API is the stable surface across any such swap
  (a windowing layer shared across backends, e.g.); the backend underneath
  it is free to change as long as the observable output stays testably
  correct (golden tests, pixel/numeric cross-checks — whatever the
  feature's own verification story is). Don't build an interim backend you
  already know will be thrown away once the better one is in scope.
- **Target what each platform is actively doing before what it used to
  do.** Scope to current OS versions/architectures first — Apple-Silicon-
  only macOS support (sidestepping the x86_64-only `objc_msgSend_stret` ABI
  split) is the current instance — and broaden to older versions of that
  same platform only once a concrete capability need justifies it, not
  preemptively as legacy-support insurance. This is a different axis from
  the backend-supersession rule above (that one swaps backends for a single
  platform; this one decides which platform/OS-version targets are in
  scope at all): it follows from the AI-native trajectory (see "What Socrates
  is for") — build for where each platform's ecosystem is actively headed,
  not for carrying yesterday's compatibility weight up front. Older
  platforms broadly are still in scope long-term; the ordering is
  capability-justified breadth, not defensive breadth-first.
- **When artifacts are consolidated or split, record the intent.** Any
  time artifacts merge or one splits, write down what each resulting
  artifact is *for*, how the pieces compose, and why the split or merge
  happened — tracked intent is the drift-prevention mechanism: an
  artifact whose purpose is written down can be checked against it, while
  one whose purpose is implied silently drifts. Recorded rationale is
  evidence, not authority — it is correctable by outside intervention
  when a conclusion proves historically wrong, and superseding it means
  recording the correction, not deleting the record. Standing instances:
  the bench files' `// Bench:` measurand headers and `bench/RESULTS.md`'s
  epoch bridge; the demos' deliberate-divergence comments; the ports'
  READMEs describing exactly what CI enforces.

## Native graphics & compute roadmap (standing directive)

**The wgpu deletion is DONE; the native build-out continues.** The
standing goal (set 2026-07-17, and easy to lose because it spans many
releases: re-read this before planning any graphics/compute work) is
native, raw-FFI, zero-dependency backends for everything — OpenGL,
OpenCL, CUDA, Vulkan, Metal, and DirectX — built over a
maximally-performant, minimal-duplication shared graphics/compute core:
the `window`/`gfx` pattern generalized (one backend-neutral Socrates-facing
surface; thin per-API backends over dlopen/`objc_msgSend`/COM raw FFI;
handle tables + enum dispatch in shared code, as `window/macos/` does in
miniature). The `wgpu`/`pollster` dependency was **deleted the same day
the coverage condition was met** (2026-07-17: Metal ✓, Vulkan ✓ compute +
graphics, OpenCL ✓ with a CI-proven real dispatch) — every build of Socrates
is now zero-dependency. CUDA compute (`src/cu.rs`, PTX via dlopen'd
libcuda), DirectX (`src/dx.rs`, D3D12/DirectCompute via COM FFI,
WARP-proven on windows runners), and the Win32 Vulkan window surface
(`src/window/win32/vulkan.rs`) all shipped in v0.8; the one item still to
build is GL-compute, if a concrete need appears. Settled decisions:

- **Sequencing:** finish the Metal arc first (PR5's graphics phases, then
  Metal *compute* reusing the same device/queue machinery), then Vulkan
  (compute, then graphics), then OpenCL / CUDA / DirectX. The shared core
  is extracted as each second-of-its-kind backend lands — abstractions
  shaped by real duplication, not guessed up front.
- **Compute shader input: SPIR-V is the lingua franca — with two
  profiles.** Vulkan, OpenCL 2.1+, and GL 4.6 ingest SPIR-V binaries
  directly, but the *format* has two compute dialects (learned when
  OpenCL landed): Vulkan/GL take the GLCompute-Logical-GLSL450 profile,
  OpenCL takes Kernel-Physical64-OpenCL (CrossWorkgroup pointer kernel
  args) — `clCreateProgramWithIL` rejects the former. The blob must match
  `gpu.backend()`; Metal (MSL), DirectX (HLSL/DXIL), and CUDA (PTX) keep
  per-backend inputs behind the same `backend_name()`-style branching
  `gfx` uses for GLSL vs. MSL. SPEC § 7.2 documents both profiles.
- **wgpu deleted only after full native coverage — executed:** the
  condition (Metal + Vulkan + one of OpenCL/DirectX at minimum) was met
  when OpenCL landed, and `wgpu`/`pollster` (and the `gpu` cargo feature
  and WGSL dialect with them) were removed the same day. `Cargo.toml`
  has no `[dependencies]` section at all; CI asserts a one-line
  `cargo tree` for the default and every feature set.

## Invariants (do not break these)

- **Zero dependencies, in every build.** `cargo tree -e normal` is one
  line for the default and for every feature set — `Cargo.toml` has no
  `[dependencies]` section at all. Features add only raw FFI to system
  libraries. (wgpu/pollster, once the sole optional exception behind a
  `gpu` feature, were deleted when native compute coverage landed — see
  the roadmap above.)
- **`docs/SPEC.md` is the source of truth.** The implementation, the golden
  tests, and the book must all agree with it; a deviation is a bug. Any
  language change updates the spec in the same PR.
- **Everything observable is golden-tested and byte-identical.** Every
  demo's full stdout is pinned (`demos/`), every ```soc block in `book/`
  executes in CI except the rare fragment fence-tagged `skip` (one today —
  135 of 136 execute), and the spec suite (`tests/spec/`, 312 tests) runs
  through the same `socrates test` path users get. A refactor that changes any
  pinned output is wrong unless the output change is the point.
- **GC-stress must stay green.** `SOCRATES_GC_STRESS=1` collects before every
  allocation; the whole suite (unit, spec, demos) passes under it. New
  natives that allocate must root correctly (see `temp_roots` in `vm.rs`).
- **Seeded randomness is stable only within a release**, never across.
  Corpora that must outlive a release use a hand-rolled PRNG, not `math.seed`.

## The gauntlet (run before shipping any interpreter change)

```sh
cargo test                                    # unit + golden spec suite
SOCRATES_GC_STRESS=1 cargo test --test spec_runner
cargo clippy --all-targets -- -D warnings
cargo build --release
./target/release/socrates test tests/spec        # 312
# glcube's three mains need a live GL/Metal/Vulkan window (CI runs them in
# the windowing jobs); everything else, cube.soc/spec.soc included:
shopt -s extglob
./target/release/socrates test demos/!(glcube)/ demos/glcube/cube.soc demos/glcube/spec.soc  # 73, also with SOCRATES_GC_STRESS=1
SOCRATES_PATH=ports ./target/release/socrates test ports/pyl/spec.soc   # + ports/icaa/spec.soc
./target/release/socrates build demos/csvql -o /tmp/csvql && (cd /tmp && ./csvql)  # `socrates build` smoke
python3 bench/ab.py <base-tree> <head-tree>   # local interleaved perf A/B
```

Performance claims are only real if the interleaved cross-binary A/B
reproduces them: `python3 bench/ab.py <base-tree> <head-tree>` locally
(each side a full checkout with its own release binary; ab.py enforces
per-rep and cross-side stdout checksums, so a wrong-answer "optimization"
fails instead of winning), and the four-arch Bench A/B workflow — push the
candidate as a `bench/<name>` branch — for the acceptance verdict, per the
universality principle: flat-or-better on every tier-1 architecture.
`bench/run.sh [N]` is single-binary sequential profiling convenience
(where does one binary spend its time?), not the gate. Method and standing
numbers: `bench/RESULTS.md`.

## Where the detailed memory lives

- `CHANGELOG.md` — per-release feature list (each release shipped as one PR).
- `docs/SPEC.md` — the normative language reference (`(vN)` tags mark when a
  feature landed).
- `docs/ARCHITECTURE.md` — implementation internals, module by module.
- `docs/RELEASING-macOS.md` — one-time setup to turn on Developer ID signing +
  notarization for the macOS demo-zoo binaries (the six repo secrets).
- `bench/RESULTS.md` — the bench method and instrument facts, the standing
  numbers, the negative-results ledger (measured and rejected — do not
  re-attempt without new evidence), the known-headroom list, and the epoch
  bridge that keeps pre-/post-re-specification numbers comparable. No
  other file holds any of these.
- `demos/NOTES.md` — the field-test triage ledgers: every papercut demo
  authors hit, and whether it was fixed / documented / declined. The raw
  material for "what usage pulled in" in a release post.
- `demos/STYLE.md` — best-practice house rules distilled from the demo
  rounds (golden discipline, determinism, bitwise, workers, std collections).
- `ports/README.md` (the programme and the `jsl` layer), `ports/pyl/CONTRACT.md`
  (the `pyl` layer's contract), and the per-port `ports/icaa/README.md` /
  `ports/claudewave/README.md` — the porting programme (SkyeShark's ICAA in
  `jsl`; claudewave in `pyl`), each README describing exactly what CI
  enforces when cross-validating that port against its upstream. (`jsl`
  has no doc file of its own; it is documented in `ports/README.md` and by
  its consumer, icaa.)
- `book/` — the language book (a teaching resource, **not** a project diary;
  process/history belongs here in `CLAUDE.md` and the files above, never in
  the book).

## Release ledger (source material for release posts)

- **v0.1** — the language: lexer, parser, unification inference with
  generics, Maranget exhaustiveness, bytecode compiler, stack VM, mark-sweep
  GC, REPL, formatter, disassembler, golden-test harness, spec, book.
- **v0.2** — impl blocks (methods on user types), the `?` operator,
  multi-file modules (`import`, diamond dedup, cycle detection), tail-call
  optimization.
- **v0.3** — `pub` visibility (private-by-default modules), operator methods
  (`add`/`sub`/…), `SOCRATES_PATH` search path, `fs`/`os` namespaces.
- **v0.4** — `socrates test` command, the embedded standard library
  (json/flags/path/strings/iter, written in Socrates), `socrates lsp`, catchable
  panics (`try`).
- **v0.5** — REPL imports, LSP completion, the book became a CI test suite.
- **v0.6 — the field-test release.** Ten demo programs written by ten
  independent authors under orders to report every papercut; ten hit the
  same dozen walls, and the release is those walls removed. One genuine RNG
  bug their tests caught: `math.seed` set state to `seed | 1`, so adjacent
  seeds `2k`/`2k+1` produced identical streams; fixed with SplitMix64
  scrambling. Triage in `demos/NOTES.md`.
- **v0.7 — the infrastructure release.** `Bytes` (packed buffers, binary
  I/O, wire formats); native `fft` namespace (radix-2 + Bluestein, numpy
  conventions, CI-cross-checked at 1e-9); OS-thread `worker` isolates with
  string channels; Int bitwise operators (`& | ^ << >>`) plus intrinsics
  (`count_ones`/`ushr`/`rotate_*`/`to_hex`) and Bytes readers/BE pushers;
  feature-gated `gpu` compute (wgpu, first optional dep, default build stays
  zero-dep); a std collections layer (`set`/`deque`/`lists`, `strings.Builder`);
  a line-width-aware formatter. Two ports validated to numeric/pixel
  equality (ICAA 18/18 pixel-exact; claudewave 32/32 battery, 29 bit-exact).
  Then a measured efficiency pass (see `bench/RESULTS.md`): checkers −15%,
  lisp −20%, string building −55%, map ops −37%, GC-stress suite −67% on the
  heaviest demo — all with byte-identical golden output. Finally, distribution:
  `socrates build` staples a program (modules, data files, worker `.soc`s) onto
  the interpreter as an appended, dependency-free payload the binary reads from
  its own tail at startup — a self-contained executable whose output is
  byte-identical to the source run. Target-independent stapling (`--launcher`)
  lets one host cross-build the whole **demo zoo**: all seventeen demos for
  `x86_64`/`aarch64` Linux + Windows and Apple-Silicon macOS, shipped in the
  release. macOS can't append (a payload past the Mach-O `__LINKEDIT` fails
  `codesign` and arm64 won't run unsigned), so there the payload is linked in
  as a `__DATA,__socrateszoo` section (`ld -sectcreate`; `socrates build
  --payload-only` emits it; `read_self` parses the running image to read it
  back) and ad-hoc signed. Developer ID signing + notarization are wired in
  `release.yml`, dormant until the `MACOS_CERT_P12_BASE64` etc. secrets exist.
- **v0.8 — native graphics and compute + the demo round's feature
  queue.** The v0.7 demo round left a
  ranked, deduplicated feature-request queue (`demos/NOTES.md`); this
  release works through it directly rather than via a fresh round.
  `if let`/`while let` (pure parser sugar, desugared fully to `match`/`while`
  at parse time — the checker and compiler need no special cases); bitwise
  compound assignment (`&= |= ^= <<= >>=`); hex/binary literals now express
  the full 64-bit pattern (bit 63 included) plus `String.parse_hex()`;
  `Bytes` 64-bit accessors (`push`/`read_u64le`/`be`); `Int.wrapping_add`/
  `sub`/`mul`; `fft.magnitude`; `Range.any`/`all`; non-blocking
  `worker.try_recv()`; `strings.Builder.is_empty`/`push_joined`;
  `lists.min_by_key`/`max_by_key`; a new `std.lazy` module (`Lazy[T]`,
  deferred/memoized computation); ergonomic `std.json` construction
  (`obj`/`arr`/`jstr`/`num`/`int`/`bool`/`null`); and `socrates test --bless`,
  which rewrites mismatched `//? expect:` lines in place when the
  actual/expected count already agrees. One item (a counting-map helper)
  declined — one demo, one-line workaround, `std` grows reluctantly. Four
  items in the original queue turned out to already be shipped in v0.7's
  own efficiency pass; `demos/NOTES.md` now says so.
  **And, in the same release, the native graphics-and-compute
  programme** — the standing roadmap directive executed end to end: `std.glm` (GLM-shaped
  vector/matrix/quaternion math, pure Socrates) + `Bytes` f32 accessors; the
  `window` namespace (OpenGL via X11/GLX, Win32/WGL, Cocoa/CGL raw FFI) and
  the GL-shaped `gfx` draw-call surface; the Metal backend (additive,
  windows + compute, MSL, interpreter moved to the macOS main thread); the
  Vulkan backend (compute + windowing + full gfx on Linux/X11 AND Windows,
  SPIR-V with in-house reflection, lavapipe-pixel-proven, everything past
  the surface in one shared `window/vulkan.rs` core); glcube rendering
  byte-identical golden frames on GL, Metal, and Vulkan; five native
  compute backends (metal/MSL, vulkan/SPIR-V, opencl/SPIR-V via
  clCreateProgramWithIL proven on Intel's CPU runtime, cuda/PTX
  graceful-proven, d3d12/HLSL WARP-proven; precedence metal > vulkan >
  d3d12 > cuda > opencl; two SPIR-V compute profiles in SPEC § 7.2); and
  the wgpu/pollster deletion the moment coverage landed — `Cargo.toml` has
  no `[dependencies]` section, `Cargo.lock` is 7 lines, CI asserts the
  one-line `cargo tree` per feature set. Shared cores extracted at each
  second consumer: `objc.rs`/`mtl.rs`, `vk.rs`, `window/vulkan.rs`
  (−1212 lines; the lavapipe asserts prove the code Windows runs). Book
  ch8 gained executable `std.glm` + `window`/`gfx` sections.
  **And, folded in before the release was ever published** (the same way
  an earlier mis-cut became this release's feature-queue half): the
  minification pass (the four-arch Bench A/B gate; the compact dispatch
  loop with the per-target `monolithic_dispatch` binding; `fft.magnitude`
  moved to `std.fft`; the math namespace minified to what only it
  provides; the `std.json` escape fast path; the demo adoption gap
  closed), the consistency passes (SPEC↔implementation reconciliation,
  ports validating exactly what CI enforces with 184 new cross-checks,
  the bench harness enforcing its own claims, the core docs re-measured
  against the current system, STYLE.md made normative with the demos
  conforming), and the rename itself — Fable became Socrates, with the
  rationale recorded atop this file and in the CHANGELOG.

## Workflow conventions

- Merge on green, by hand: feature PRs are real (non-draft) — drafts are
  reserved for *releases* — and `main` carries a required status check,
  "Test (stable)", so a red PR cannot merge. Merges are performed
  manually after reading the decisive CI log, not by arming auto-merge;
  a change that touches the interpreter or `bench/` is additionally
  gated on a clean four-arch Bench A/B matrix verdict. Feature work
  happens on a dedicated branch off `main`.
- Commit messages state what changed and (for perf) the measured delta.
- The spec, the book's executable snippets, and the demos' pinned output are
  the three tripwires — if a change is wrong, one of them goes red.
- **CHANGELOG, book, README, and ARCHITECTURE updates happen in-session,
  before the PR — not a separate follow-up "docs pass" PR after the fact.**
  This matches `docs/SPEC.md`'s own same-PR rule (see Invariants) and now
  extends to the rest of the documentation set: validate what you write
  (run the book-snippet suite, re-check any counts/numbers you cite) before
  shipping, in the same session that made the change, not after. A
  dedicated later docs-pass PR is the exception for a batch of already-
  shipped feature PRs that predate this rule, not the default going forward.
