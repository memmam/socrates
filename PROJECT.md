# Socrates — what the project is, and how it decides

Socrates is a statically-typed, garbage-collected programming language with
algebraic data types, exhaustive pattern matching, closures, and generics,
implemented from scratch in Rust with **zero dependencies** — every build.
This file holds everything Socrates-specific that isn't about how to
operate a session: what the language is *for*, the engineering principles
that decide close calls, the native graphics/compute roadmap, and the
invariants that must never break. `CLAUDE.md` holds session-operating
instructions only (the file map, the verification gauntlet, git/PR/session
workflow) and explicitly says to check this file wherever an operating
step needs it. `HISTORY.md` holds the incident narratives behind the
rules in both files; `CHANGELOG.md` holds the per-release account.

## What Socrates is for

**The name:** Socrates, formerly Fable (full rationale in `CHANGELOG.md`'s
v0.8.0 entry, "Renamed"). `bench/ab.py` and the Bench A/B workflow carry
a permanent cross-name fallback that keeps pre-rename refs benchable.

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
  macOS — the four-arch bench matrix; the release matrix adds a fifth,
  aarch64 Windows, which the bench workflow doesn't cover), not on one
  box: simplifying an idiom down
  to pure primitives can run better on one architecture and worse on
  another (I-cache geometry, indirect-branch cost, and code layout all
  vote differently per arch). A simplification is accepted only if the
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
  faster and inlines them back). And the search for that per-target
  idiom can itself terminate at the uniform form: when every
  finer-grained binding measures worse — probe evidence showing the
  implicated piece is load-bearing elsewhere on the same target — the
  uniform form stands as that target's measured-fastest, and the
  residual row is recorded with the probe receipts in
  `bench/RESULTS.md`, never waived silently.
- **A verdict needs ≥5 samples, no exception, before it is final** —
  every CI-matrix leg and every local single-box probe's keep/drop call.
  Fewer than 5 samples is an inconclusive result, not a negative one; it
  does not license a DROP or a dismissal, only more sampling.
  **The floor is a floor, not a ceiling reached by counting: when 5
  samples don't converge, escalate the *kind* of evidence, not the
  count.** Wall-clock A/B on a shared runner has an irreducible noise
  source; past the point that noise dominates a marginal signal, a 6th
  same-kind sample just re-measures the same noise. The next probe
  changes what's measured — a deterministic instrument immune to
  scheduler jitter (instruction/cache counts), or escalation to an
  entity outside the automated loop (the user) — not the sample count.
  The full protocol, case law, and revalidation notes on verdicts that
  predate this floor live in `bench/RESULTS.md`; HISTORY.md's own
  incident entry for the floor's history also states it, as part of
  that narrative rather than as a second normative restatement — both
  are deliberate per the intent-tracking principle's scope-recording
  discipline, not drift.
  **The deterministic-instrument branch is itself a ladder, not one
  shot:** hypothesis → a test built to confirm or refute it
  specifically → confirmed (commit, scope the idiom set up) or refuted
  (the next hypothesis, tested the same way) — bounded at four
  hypothesis-tests before a fifth candidate with none confirmed is
  itself the signal to take the other branch (escalate to the user).
  A running scratchpad of each test's data feeds a slot-by-slot rule for
  what to spend each probe or sample on: (1) first — ground; (2) second
  — differential, never enough alone to confirm or reject; (3) third —
  probe/sample for something else IFF that would give better insight
  than reprobing the same condition, else reprobe/resample it; (4)
  fourth — if compelled, test here (an early exit into the test
  itself); otherwise the same choice as step 3, with the target
  abandoned in step 3 eligible again; (5) fifth — decisive: test if
  motivated, and either commit to the hypothesis and scope the idiom set
  up, or change hypothesis, based on the accreted evidence. This lets a
  hypothesis be dropped *or* promoted early on partial data, for
  navigating between hypotheses faster — never for the underlying
  KEEP/DROP verdict itself, which still needs its own full ≥5 once a
  hypothesis is confirmed. Full protocol, spelled out slot by slot, in
  `bench/RESULTS.md`.
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
  capability-justified breadth, not defensive breadth-first (recorded
  long-term instance: Intel-based Mac as a potential legacy target).
  The mirror-image rule for things currently supported: **deprecated is
  not discontinued** — a platform or CI image marked deprecated counts
  as supported in practice until its actual removal date, and interim
  brownouts or discrepancies are cause to *expand* coverage to span old
  and new (the macos-14/macos-15 dual CI rows are the first instance;
  the 14 rows retire 2026-11-02 when GitHub actually removes the image),
  never cause to early-retire the older one.
- **The intent-tracking principle: when artifacts are consolidated or
  split, record the intent.** Any
  time artifacts merge or one splits, write down what each resulting
  artifact is *for*, how the pieces compose, and why the split or merge
  happened — tracked intent is the drift-prevention mechanism: an
  artifact whose purpose is written down can be checked against it, while
  one whose purpose is implied silently drifts. Recorded rationale is
  evidence, not authority — it is correctable by outside intervention
  when a conclusion proves historically wrong, and superseding it means
  recording the correction, not deleting the record. And record
  decisions with their *scope* — what exactly was approved, no wider:
  an overbroad memory of a narrow decision is how conventions drift. And
  codification itself is a four-step act, not a sentence — a rule that
  gets codified:
  1. lands in the repo file where it operationally binds;
  2. lands here with its scope and first instance;
  3. is copied into the session's working memory;
  4. triggers an immediate consistency audit of the existing tree and
     policies *against the new rule* — retroactive application is part
     of codifying, because a forward-only rule leaves its whole class
     dirty behind it.

  Standing
  instances of tracked intent: the bench files' `// Bench:` measurand
  headers and `bench/RESULTS.md`'s epoch bridge; the demos'
  deliberate-divergence comments; the ports' READMEs describing exactly
  what CI enforces; and RESULTS.md's standing-watch entries — a rejected
  change that keeps its re-open condition attached to the record, so the
  correction path is itself part of the memory rather than a fact
  someone must rediscover.
  **The workaround-recording triple** is the content rule the
  four-step act doesn't cover on its own: any recorded manual rule or
  workaround states (i) the official mechanism it bypasses, (ii) the
  blocker that forced the bypass, and (iii) a dated condition under
  which the blocker gets re-checked — so the record can't silently
  outlive its own premise. The empty-commit bench-resampling method is
  the model (bypasses `workflow_dispatch`/rerun; blocked by both
  returning 403 for the bot account; re-check on any App permission
  change). A workaround recorded without the triple is exactly the
  gap the reflexive codification audit exists to catch.
- **`std` surface is earned, never speculative, and named for what it
  actually does.** `std` grows reluctantly — the rule `demos/NOTES.md`
  established across the v0.6/v0.7 rounds, restated here as project-wide
  rather than demos-local: a module ships only the operations a real
  caller needs right now, not the operations that would make it feel
  complete. A decode/round-trip/reverse of an encode-only need, with no
  consumer beyond its own test, is speculative surface — cut it, don't
  keep it "for symmetry." The same reluctance runs backward, not just
  forward: a demo that has become strictly a self-contained format/file
  generator, with no demo-specific logic left in it, is a promotion
  candidate the moment an audit notices it, not something grandfathered
  because it already shipped — split it into the smallest reusable
  pieces rather than moving it wholesale, and turn any call-site-specific
  tuning it hardcoded (a block size, a buffer constant) into an explicit
  parameter rather than a std-level default, so the promotion changes
  nothing about what any existing caller produces. And a name is a
  claim: a std/public function or module named after a well-known
  algorithm or format must actually do what that name implies — if it
  isn't literally what the name says (a "deflate" that never runs
  LZ77/Huffman, say), rename it to what it actually is or coin a name
  that doesn't collide, rather than let the name overclaim. The same
  rule runs forward, not just backward: before deliberately changing
  what a named function or module does, rename it first, rather than
  let the old name start meaning something different under running
  programs' feet — and if any caller genuinely depends on conformance
  to the *original* named spec, not just the name, that conformance
  needs its own explicit, correct implementation, never a hope that the
  renamed or evolving one eventually gets there. First instances of all
  three: the `std.wav` decode-trim, the six-module
  demo→std promotion wave, and `zlib.deflate_stored`/`inflate_stored` →
  `wrap`/`unwrap` — `HISTORY.md` has the story.

## Native graphics & compute roadmap (standing directive)

**The wgpu deletion is DONE; the native build-out continues.** The
standing goal (easy to lose because it spans many releases: re-read this
before planning any graphics/compute work) is native, raw-FFI,
zero-dependency backends for everything — OpenGL, OpenCL, CUDA, Vulkan,
Metal, and DirectX — built over a maximally-performant,
minimal-duplication shared graphics/compute core: the `window`/`gfx`
pattern generalized (one backend-neutral Socrates-facing surface; thin
per-API backends over dlopen/`objc_msgSend`/COM raw FFI; handle tables +
enum dispatch in shared code, as `window/macos/` does in miniature).
Metal, Vulkan (compute + graphics), OpenCL, CUDA compute (`src/cu.rs`),
DirectX (`src/dx.rs`), and the Win32 Vulkan window surface
(`src/window/win32/vulkan.rs`) have all shipped; `wgpu`/`pollster` were
deleted as soon as Metal + Vulkan + OpenCL landed (the minimum coverage
condition, met before CUDA/DirectX/the Win32 Vulkan window shipped — see
the "wgpu deleted only after full native coverage" bullet below for the
exact condition and `HISTORY.md` for how the rollout sequenced), so
every build of Socrates is zero-dependency. The one item still to build
is GL-compute, if a concrete need appears.
Settled decisions:

- **Sequencing:** finish the Metal arc first (the Metal window/gfx
  surface's phased rollout, then Metal *compute* reusing the same
  device/queue machinery), then Vulkan
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
  135 of 136 execute), and the spec suite (`tests/spec/`, 313 tests) runs
  through the same `socrates test` path users get. A refactor that changes any
  pinned output is wrong unless the output change is the point.
- **GC-stress must stay green.** `SOCRATES_GC_STRESS=1` collects before every
  allocation; the whole suite (unit, spec, demos) passes under it. New
  natives that allocate must root correctly (see `temp_roots` in `vm.rs`).
- **Seeded randomness is stable only within a release**, never across.
  Corpora that must outlive a release use a hand-rolled PRNG, not `math.seed`.
