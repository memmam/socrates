# Socrates — project memory

Socrates is a statically-typed, garbage-collected programming language with
algebraic data types, exhaustive pattern matching, closures, and generics,
implemented from scratch in Rust with **zero dependencies** — every build.
This file is the working memory for the project: what Socrates is *for*,
the invariants that must never break, the engineering principles that
decide close calls, and how to verify a change — everything needed to
operate a session and regenerate correctly if a container is refreshed or
a model swaps. `HISTORY.md` holds the rest: the incidents that motivated a
rule, the sagas behind a corrected decision, and the per-release ledger.

## What Socrates is for

**The name:** Socrates, formerly Fable (full rationale in `HISTORY.md`).
`bench/ab.py` and the Bench A/B workflow carry a permanent cross-name
fallback that keeps pre-rename refs benchable.

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
  predate this floor live in `bench/RESULTS.md` — this is the one
  other file that states the number, per the intent-tracking
  principle's scope-recording discipline.
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
- **When artifacts are consolidated or split, record the intent.** Any
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
  codification itself is a four-step act, not a sentence: a rule that
  gets codified (i) lands in the repo file where it operationally binds,
  (ii) lands here with its scope and first instance, (iii) is copied
  into the session's working memory, and (iv) triggers an immediate
  consistency audit of the existing tree and policies *against the new
  rule* — retroactive application is part of codifying, because a
  forward-only rule leaves its whole class dirty behind it. Standing
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
  outlive its own premise. `cleanup.yml`'s release-by-PR pattern is
  the model (bypasses `git push --delete`; blocked by the App's
  branch-scoped credentials; re-check whenever the App's token scope
  changes) and the empty-commit bench-resampling method is a second
  instance (bypasses `workflow_dispatch`/rerun; blocked by both
  returning 403 for the bot account; re-check on any App permission
  change). A workaround recorded without the triple is exactly the
  gap the reflexive codification audit exists to catch.

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
deleted once that coverage landed, so every build of Socrates is
zero-dependency (see `HISTORY.md` for how the rollout sequenced). The
one item still to build is GL-compute, if a concrete need appears.
Settled decisions:

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
  135 of 136 execute), and the spec suite (`tests/spec/`, 313 tests) runs
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
./target/release/socrates test tests/spec        # 313
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
fails instead of winning — and warns if the two checkout paths are
unequal length, since that alone shifts binary layout; use equal-length
directory names, e.g. `base/` and `head/`), and the four-arch Bench A/B
workflow — push the candidate as a `bench/<name>` branch — for the
acceptance verdict, per the
universality principle: flat-or-better on every tier-1 architecture.
`bench/run.sh [N]` is single-binary sequential profiling convenience
(where does one binary spend its time?), not the gate. Method and standing
numbers: `bench/RESULTS.md`.

## Where the detailed memory lives

- `HISTORY.md` — the historical half of this file: incident narratives,
  sagas behind corrected decisions, and the per-release ledger. Read it
  when a rule here points to it, when writing a release post, or when
  auditing whether a rule still matches the incident that produced it.
- `CHANGELOG.md` — per-release feature list (each release shipped as one PR).
- `docs/SPEC.md` — the normative language reference (`(vN)` tags mark when a
  feature landed).
- `docs/ARCHITECTURE.md` — implementation internals, module by module.
- `docs/RELEASING-macOS.md` — one-time setup to turn on Developer ID signing +
  notarization for the macOS demo-zoo binaries (the six repo secrets).
- `bench/RESULTS.md` — the bench method and instrument facts, the standing
  numbers, the negative-results ledger (measured and rejected — do not
  re-attempt without new evidence; an entry may instead carry a
  **standing watch**: dated sightings of its trigger signal accumulate
  in the entry itself, and enough of them across genuinely different
  cases re-opens the item — the inline-small-list entry is the first),
  the known-headroom list, and the epoch
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
  process/history belongs here in `CLAUDE.md`/`HISTORY.md` and the files
  above, never in the book).

Each of the four directories above (`docs/`, `bench/`, `demos/`, `ports/`)
also gets a nested per-directory `CLAUDE.md` stub (bare filename — nested
`.claude/CLAUDE.md` is not a real discovery path; that's reserved for
settings/skills/rules) that does nothing but `@`-import the file(s)
already listed for it above, so Claude Desktop's context-tracker "Memory
files" panel lists `docs/SPEC.md` and friends as their own entries. The
`@`-import is not lazy about *content* — the stub force-loads the entire
imported file(s) the moment it fires, a real, compounding cost paid by
every clone and contributor. The four stubs are committed, tracked
files, byte-exact with the table below — reconstructing them from a
one-example-plus-inference description is exactly the kind of drift this
file exists to prevent. (`HISTORY.md` has the story of how this mechanism
evolved — it started gitignored and opt-in before being proven and
committed.)

| File | Content |
| --- | --- |
| `docs/CLAUDE.md` | `@SPEC.md` / `@ARCHITECTURE.md` / `@RELEASING-macOS.md` |
| `bench/CLAUDE.md` | `@RESULTS.md` |
| `demos/CLAUDE.md` | `@NOTES.md` / `@STYLE.md` |
| `ports/CLAUDE.md` | `@README.md` / `@pyl/CONTRACT.md` / `@icaa/README.md` / `@claudewave/README.md` |

(Each `/`-separated entry is its own line in the file, in that order.)

## Workflow conventions

- Merge on green, by hand: feature PRs are real (non-draft) — drafts are
  reserved for *releases* — and `main` carries a required status check,
  "Test (stable)", so a red PR cannot merge. Merges are performed
  manually after reading the decisive CI log, never on a green
  conclusion alone — and which log is decisive is tiered by what the
  change risks: a perf-bearing change reads the four matrix tables and
  the Test log, an interpreter change reads the Test log's suite counts
  and port batteries, a prose-only change reads the Test log tail. Not
  by arming auto-merge. A change that touches the interpreter or the
  bench *sources or harness* (`bench/*.soc`, `ab.py`, `run.sh`,
  `bench.yml`) is additionally gated on a clean four-arch Bench A/B
  matrix verdict — `RESULTS.md` prose is exempt (it changes no binary;
  its matrix run would be an A/A that tests nothing). The verdict
  attaches to the tree that built the judged binaries: follow-up
  commits that touch no compiled source (docs, prose) ride the
  existing verdict without a re-run. Feature work happens on a
  dedicated branch off `main`.
- Non-landing work is pushed for durability without a PR: a dropped
  probe or a held wave lives on its own pushed branch rather than being
  discarded or forced into a PR that was never going to merge. PRs are
  for changes meant to merge, drafts for releases, archival branches for
  neither.
- Commit messages state what changed and (for perf) the measured delta,
  and end with the two attribution trailers (`Co-Authored-By` and the
  `Claude-Session` link) — the accepted channel for session
  attribution, and the *only* one.
- The spec-suite count is stated in exactly five places — `README.md`
  (×2), this file (×2), and `.github/RELEASE_NOTES.md` — and a count
  change updates all five in the same PR. The same discipline covers
  every other prose-stated count — book snippets executed/total, the
  demo-golden count, the spelled-out demo-program count — each with its
  own set of stating places. `tools/check_counts.sh` (run in CI's Test
  job) is the enforcement: it extracts every counted sentence by exact
  anchor and diffs it against a fresh run, so drift fails loudly instead
  of shipping; a sentence reworded without updating its anchor fails
  just as loudly ("anchor matched nothing"), which is the intended
  fail-closed behavior — re-anchor in the same PR that reworks the prose.
- **A fixed target does not rot.** When CI fails on a pinned, fixed
  artifact — a runner image, an action pinned by SHA, a vendored blob —
  the artifact is the *last* suspect: the failure is almost always the
  DNS/access/infrastructure layer around it, and even scheduled
  "brownouts" are access denials imposed on a still-working image, not
  material failures of it. Diagnose by reading the log for the infra
  signature first (DNS resolution, download retries, 403/429, runner
  provisioning); the remedy ladder is retry — a failed run never
  restarts itself, so act immediately: push or empty-commit re-fire —
  then user-level intervention for persistent access/policy failures.
  Re-scoping or retiring the fixed target is never the inferred fix.
- **Session mechanics — durable on purpose.** Rules that lived only in
  session memory kept getting dropped between sessions (session ledgers
  die with their containers), so they live here now; a session ledger
  may carry working copies, but this list is the source. Where a rule
  touches a hosted-tooling default, it is written to *compose with*
  the default rather than fight it — fighting defaults is how the
  triple-footer happened. The rules:
  - Every git-mutating shell command opens
    `cd <dir> && pwd && git branch --show-current`.
  - If a session ever holds more than one clone of the repo, the
    harness-served clone is pulled after every CLAUDE.md-touching
    merge until the checkouts are consolidated.
  - Branches are deleted only in user-directed cleanups, never
    unilaterally. Before a cleanup, anything a standing record
    references moves to an `archive/*` branch. The App's credentials
    can create refs but not delete them, so deletions run through the
    release-by-PR pattern: a user-directed PR edits
    `.github/CLEANUP_BRANCHES`, and `cleanup.yml` (contents: write;
    refuses `main`, `archive/*`, `claude/*`) performs the deletions
    when the change lands on main.
  - A merge the user performs in the GitHub UI is a final outcome,
    never something to re-adjudicate.
  - Never run bare `cargo fmt` — the tree has never been through it,
    so `cargo fmt --check` diffs every `.rs` file, not a targeted few.
    Run `cargo fmt --check | grep -c '^Diff in'` for the current figure
    rather than trusting a number written down here, since it drifts as
    the tree grows; the point is that it's whole-tree, not the exact
    count.
  - The model identifier appears in no pushed artifact — chat only
    (this restates the hosted-environment policy so the rule survives
    outside it). This includes the commit-trailer `Co-Authored-By`
    *name*, not just prose — the trailer name is plain `Claude`,
    nothing else, going forward; past commits are not rewritten
    retroactively for this.
  - Decision forks go to the user as plain-text lettered options, not
    interactive question UI.
  - Long CI waits are handled by scheduled self check-ins, never
    polling loops — that's this session's own wakeup mechanism, not
    available to a delegated subagent. A subagent briefed to push,
    wait on a bench/CI run, and continue has no independent wakeup of
    its own: told to just "wait," it ends its turn and stalls, needing
    a manual resume every single time. The fix for that case is the
    mirror image of this rule, not an exception to it: a subagent
    waiting on a run polls *within its own turn*, a bounded bash loop
    (`sleep` + a status check, capped at enough iterations to cover one
    run), and only ends its turn once it has an actual result or has
    exhausted the cap — never on a bare "standing by."
  - **A signal is a prompt to check, not a substitute for checking.**
    A task-notification, webhook event, or elapsed check-in interval
    means "go verify the actual state now" — not "the state is
    whatever the signal implies." When a check comes back ambiguous,
    empty, or merely "not yet," the next move is a *different, more
    direct* query against the actual repo/CI state — go to the source
    of truth, don't wait on the signal layer to resolve itself — not
    another wait cycle.
  - **A wakeup firing is never a terminal, silent event.** Every
    scheduled check-in resolves in exactly one of two states, in
    order: (1) check whatever pending status it was armed for
    directly, act on what's found, and — if work still remains —
    schedule the next wakeup before the turn ends, so the loop never
    dies silently between firings; (2) if nothing further is
    automatable (a decision needs the user, or the work is genuinely
    done), end the turn by saying so explicitly, never with just tool
    calls and no wakeup and no closing status.
  - **Anything structural or behavioral about how memory itself is
    configured** — CLAUDE.md conventions, what gets committed,
    nested-stub patterns and their exact content — lands here,
    byte-exact, the same session it's decided, same as any other rule
    on this list. This is that rule applied to itself: Claude Code's
    own auto-memory ("memories folder",
    `~/.claude/projects/<project>/memory/`) is explicitly machine-local
    and does not survive a fresh container or a different session's
    checkout, and a scratchpad session ledger is even more ephemeral
    (gone the moment its container is reclaimed) — either one is a
    *reconstruction* source, not a durable one, and reconstructing from
    a partial or ambiguous description (one example generalized by
    inference, say) is where drift creeps in between sessions or across
    a model swap.
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
