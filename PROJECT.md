# Socrates — what the project is, and how it decides

Socrates is a statically-typed, garbage-collected programming language with
algebraic data types, exhaustive pattern matching, closures, and generics,
implemented from scratch in Rust with **zero dependencies** — every build.
This file holds everything Socrates-specific: what the language is *for*,
the engineering principles that decide close calls, the native
graphics/compute roadmap, the invariants that must never break, and —
since `CLAUDE.md` states its own rules generically and points here for the
concrete fill-in — this project's file map, its nested-stub table, its
verification gauntlet, its counted places and checker script, its
workflow specifics, and its tripwires. `CLAUDE.md` holds only the
universal, project-agnostic rules (session mechanics, git/PR/workflow
conventions) that hold regardless of what project they're checked into,
and its own opening paragraph says where `HISTORY.md` and
`CHANGELOG.md` fit.

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

## Where this project's memory lives

- `CLAUDE.md` — universal session-operating rules: session mechanics and
  git/PR/workflow conventions that hold regardless of what project they're
  checked into.
- `PROJECT.md` (this file) — what Socrates is *for*, the engineering
  principles that decide close calls, the native graphics/compute roadmap,
  the invariants that must never break, and the concrete fill-in for every
  generic rule CLAUDE.md states abstractly: the file map below, the
  nested-stub table below, the verification gauntlet, the counted places
  and checker script, the workflow specifics, and the tripwires.
- `HISTORY.md` — the incident narratives and sagas behind the rules and
  corrected decisions in this file and CLAUDE.md. Check it when a rule
  points here, or when auditing whether a rule still matches the incident
  that produced it.
- `CHANGELOG.md` — the per-release account: feature lists, benchmark
  deltas, and mechanism detail, one `## vX.Y.Z` heading per release with
  one bullet per feature/fix underneath — the entry is the unit of
  account, not the PR count behind it (see HISTORY.md for how that
  stopped holding at v0.8). Check it for release-post material or the
  full story behind any rename or shipped feature a rule only mentions
  in passing. **Once a release is git-tagged, its entry is historical
  record, not a live draft: a factual error found later gets a dated,
  explicit appended correction, never a silent in-place rewrite** — the
  same discipline HISTORY.md applies to its own incident narratives.
  Only the current untagged section (the one still being written toward
  the next tag) is freely editable.
- `docs/SPEC.md` — the normative language reference (`(vN)` tags mark
  when a feature landed).
- `docs/ARCHITECTURE.md` — implementation internals, module by module.
- `docs/RELEASING-macOS.md` — one-time setup to turn on Developer ID
  signing + notarization for the macOS demo-zoo binaries (the six repo
  secrets).
- `bench/RESULTS.md` — the bench method and instrument facts, the
  standing numbers, the negative-results ledger (measured and rejected —
  do not re-attempt without new evidence; an entry may instead carry a
  **standing watch**: dated sightings of its trigger signal accumulate in
  the entry itself, and enough of them across genuinely different cases
  re-opens the item — the inline-small-list entry is the first), the
  known-headroom list, and the epoch bridge that keeps
  pre-/post-re-specification numbers comparable. No other file holds any
  of these.
- `demos/NOTES.md` — the field-test triage ledgers: every papercut demo
  authors hit, and whether it was fixed / documented / declined. The raw
  material for "what usage pulled in" in a release post.
- `demos/STYLE.md` — best-practice house rules distilled from the demo
  rounds (golden discipline, determinism, bitwise, workers, std
  collections).
- `ports/README.md` (the programme and the `jsl` layer),
  `ports/pyl/CONTRACT.md` (the `pyl` layer's contract), and the per-port
  `ports/icaa/README.md` / `ports/claudewave/README.md` — the porting
  programme (SkyeShark's ICAA in `jsl`; claudewave in `pyl`), each README
  describing exactly what CI enforces when cross-validating that port
  against its upstream. (`jsl` has no doc file of its own; it is
  documented in `ports/README.md` and by its consumer, icaa.)
- `book/` — the language book (a teaching resource, **not** a project
  diary; process/history belongs in `CLAUDE.md`, `PROJECT.md`,
  `HISTORY.md`, or `CHANGELOG.md`, never in the book).

CLAUDE.md describes the nested-stub mechanism generically; this is the
concrete table for the four directories it applies to (`docs/`,
`bench/`, `demos/`, `ports/`) — which one imports which file(s), in
which order. The four stubs are committed, tracked files; the
load-bearing part is byte-exact with this table. (`HISTORY.md` has the
story of how this mechanism evolved — it started gitignored and opt-in
before being proven and committed.)

| File | Content |
| --- | --- |
| `docs/CLAUDE.md` | `@SPEC.md` / `@ARCHITECTURE.md` / `@RELEASING-macOS.md` |
| `bench/CLAUDE.md` | `@RESULTS.md` |
| `demos/CLAUDE.md` | `@NOTES.md` / `@STYLE.md` |
| `ports/CLAUDE.md` | `@README.md` / `@pyl/CONTRACT.md` / `@icaa/README.md` / `@claudewave/README.md` |

(Each `/`-separated entry is its own line in the file, in that order.)

## The verification gauntlet

Run before shipping any change that touches the interpreter's core logic:

```sh
cargo test                                    # unit + golden spec suite
SOCRATES_GC_STRESS=1 cargo test --test spec_runner
cargo clippy --all-targets -- -D warnings
cargo build --release
./target/release/socrates test tests/spec        # 314
# glcube's three mains need a live GL/Metal/Vulkan window (CI runs them in
# the windowing jobs); everything else, cube.soc/spec.soc included:
shopt -s extglob
./target/release/socrates test demos/!(glcube)/ demos/glcube/cube.soc demos/glcube/spec.soc  # 68
SOCRATES_GC_STRESS=1 ./target/release/socrates test demos/!(glcube)/ demos/glcube/cube.soc demos/glcube/spec.soc
SOCRATES_PATH=ports ./target/release/socrates test ports/pyl/spec.soc
SOCRATES_PATH=ports ./target/release/socrates test ports/icaa/spec.soc
./target/release/socrates build demos/csvql -o /tmp/csvql && (cd /tmp && ./csvql)  # `socrates build` smoke
python3 bench/ab.py <base-tree> <head-tree>   # local interleaved perf A/B
```

Performance claims are only real if the interleaved cross-binary A/B
reproduces them: `python3 bench/ab.py <base-tree> <head-tree>` locally
(each side a full checkout with its own release binary; ab.py enforces
per-rep and cross-side stdout checksums, so a wrong-answer "optimization"
fails instead of winning — and warns if the two checkout paths are
unequal length, since that alone shifts binary layout; use equal-length
directory names, e.g. `base/` and `head/`), and the four-arch Bench A/B
workflow — push the candidate as a `bench/<name>` branch — for the
acceptance verdict, per this file's own universality principle (below):
flat-or-better on every tier-1 architecture. `bench/run.sh [N]` is
single-binary sequential profiling convenience (where does one binary
spend its time?), not the gate. Method and standing numbers:
`bench/RESULTS.md`.

CLAUDE.md names golden/pinned test surfaces as this project's
tripwires; the three are the spec suite (314 golden tests under
`tests/spec/`, run through the same `socrates test` path users get), the book's
executable snippets (every ```soc block in `book/` runs in CI except
the rare fragment fence-tagged `skip`), and the demos' pinned output
(every demo's full stdout is golden-tested, `demos/`).

Any prose-stated count that could silently drift out of sync with
reality is stated in a fixed set of places, all updated in the same PR,
and checked by `tools/check_counts.sh` (run in CI's Test job and by the
gauntlet locally): the spec-suite count in exactly six places —
`README.md` (×2), this file (×2: the gauntlet script above and the
invariants section below), `.github/RELEASE_NOTES.md` (×1), and
`book/11-toolchain.md` (×1). The same discipline covers every other
prose-stated count — book snippets executed/total, the demo-golden
count, the spelled-out demo-program count — each with its own set of
stating places. The checker extracts every counted sentence by exact
anchor and diffs it against a fresh run, so drift fails loudly instead
of shipping; a sentence reworded without updating its anchor fails just
as loudly ("anchor matched nothing") — re-anchor in the same PR that
reworks the prose.

Never trust a bare `cargo fmt --check` count without re-measuring: this
tree has never been run through a bare `cargo fmt`, so
`cargo fmt --check | grep -c '^Diff in'` diffs every `.rs` file, not a
targeted few — run it fresh rather than trusting a number written down
anywhere, since it drifts as the tree grows.

## Workflow conventions (this project's specifics)

The default branch (`main`) carries one required status check, "Test
(stable)" — a red PR cannot merge. Reading tiers for the decisive CI log
before a manual merge: a perf-bearing change reads the four-arch Bench
A/B matrix tables plus the Test log; an interpreter change reads the
Test log's suite counts and port batteries; a prose-only change reads
the Test log tail. Auto-merge is fine specifically for the last tier —
prose/config-only, no interpreter or bench source touched — because
`tools/check_counts.sh`, running inside that same Test job, already
substitutes for the manual read that tier requires.

A change that touches `bench/*.soc`, `ab.py`, `run.sh`, or `bench.yml`
(the bench sources or harness) is gated the same way a core-logic change
is (see the universality principle, below): a clean four-arch Bench A/B
matrix verdict, obtained by pushing the candidate as a `bench/<name>`
branch. `bench/RESULTS.md` prose-only edits are exempt (they change no
binary; a matrix run on them would be an A/A that tests nothing). The
verdict attaches to the tree that built the judged binaries: follow-up
commits touching no compiled source ride the existing verdict without a
re-run.

This project currently names no standing permanent exceptions: the
steady state on origin really is just `main` and the single reused
`claude/*` worker branch (verified live via `git ls-remote --heads
origin` — nothing else exists there right now). `archive/*` and a
`bench/<name>` "never merges" probe branch look like exceptions while
they're active, but neither is actually permanent — both are temporary
holding categories, retired the moment their content becomes
self-sufficient in a standing-results file (`bench/RESULTS.md`,
`HISTORY.md`) rather than kept around as the record itself. The one
`archive/*` instance to date, `archive/h2-small-list`, was deleted the
same day its finding was written up; see HISTORY.md's `h3-probe-no-glc`
incident for the same logic applied to a probe branch.

Frozen-copy examples (the stability-required exception to "point to a
fact, don't duplicate it"): `ports/claudewave/reference/sos_freeze.txt`
(`ports/pyl/CONTRACT.md`'s coefficient freeze — a golden-pinned value
that must not silently track a scipy upstream that might reformulate
its algorithm) and every `bench/*.soc` file's `// Bench:` measurand
header (states what the row measures so re-specifying the workload is a
deliberate, tracked act, per the intent-tracking principle below).

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
  135 of 136 execute), and the spec suite (`tests/spec/`, 314 tests) runs
  through the same `socrates test` path users get. A refactor that changes any
  pinned output is wrong unless the output change is the point.
- **GC-stress must stay green.** `SOCRATES_GC_STRESS=1` collects before every
  allocation; the whole suite (unit, spec, demos) passes under it. New
  natives that allocate must root correctly (see `temp_roots` in `vm.rs`).
- **Seeded randomness is stable only within a release**, never across.
  Corpora that must outlive a release use a hand-rolled PRNG, not `math.seed`.
