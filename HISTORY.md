# Socrates — project history

`CLAUDE.md` is kept lean on purpose: what's needed to operate a session,
follow the engineering principles as meant, and regenerate correctly if
a container is refreshed or a model swaps. This file is where the
*historical* half of that content lives instead — the incidents that
motivated a rule, the sagas behind a corrected decision, and the
per-release ledger. Nothing here is required to operate correctly today;
it's the evidence trail for anyone who wants to know why a rule reads
the way it does, or material for writing a release post.

Read `CLAUDE.md` first. Come here when a rule's own text points here,
or when writing a release post, or when auditing whether a rule still
matches the incident that produced it.

## The rename: Fable → Socrates

Recorded 2026-07-18. Trademark pre-emption and namespace collisions —
an established F#-to-JS compiler already holds "Fable". "Socrates"
names the language's substrate role; "Timaeus" was considered and is
reserved for the eventual top-of-stack agent; "Quine" was considered
and rejected (an existing OSS graph database holds it). The `.soc`
extension nods at the system-on-a-chip trajectory of the HDL roadmap.
Git history preserves the old name; `bench/ab.py` and the Bench A/B
workflow carry a permanent cross-name fallback that keeps pre-rename
refs benchable (that fallback is the one operationally-relevant fact,
and it's stated in `CLAUDE.md` itself).

## Release ledger

Per-release account of what shipped, in order. Source material for
release posts and changelog entries; `CHANGELOG.md` is the terser,
per-PR-mapped sibling of this list.

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
  closed; the `List.sum()` native with `lists.sum` as its one-line
  wrapper; four VM superinstructions fusing the hottest operand-fetch
  pairs, with jump-target-safe fusion after patching), the consistency
  passes (SPEC↔implementation reconciliation,
  ports validating exactly what CI enforces with 184 new cross-checks,
  the bench harness enforcing its own claims, the core docs re-measured
  against the current system, STYLE.md made normative with the demos
  conforming), and the rename itself — Fable became Socrates, with the
  rationale recorded above and in the CHANGELOG.

## Engineering-principle incidents

The rules these motivated are stated plainly in `CLAUDE.md`; this is the
evidence trail behind each.

- **The dispatch-loop codegen lottery.** The recorded instance behind
  "universality gates minification": simplifying an idiom down to pure
  primitives measured differently per architecture (I-cache geometry,
  indirect-branch cost, and code layout all vote differently per arch),
  which is why a per-target binding (`monolithic_dispatch`) exists rather
  than a single uniform form.
- **The superinstruction wave's aarch64-macos `for_range` row** — the
  first instance of "when every finer-grained binding measures worse,
  the uniform form stands as that target's measured-fastest, and the
  residual is recorded, never waived silently." Full receipts in
  `bench/RESULTS.md`.
- **The ≥5-sample floor's history.** Raised 2026-07-18 from a ≥3 macOS
  floor that carried a same-decision escape hatch down to two samples;
  the escape hatch is what let a real mark get dismissed on 1-of-2 — the
  W2 `enum_match` errata in `bench/RESULTS.md` is the recorded instance.
  No floor at all existed for local probes before this, which had
  informally run on two samples every time.
- **The sixth-probe doctrine's first instances:** `bench/h3-probe-no-glc`
  (mechanism isolation, recognized as this pattern only after the fact)
  and `bench/h1-binding-recheck` (the first deliberate instance).
- **The hypothesis-test ladder's first instance:**
  `bench/inline-upvals-x64-probe`, testing PR #103's x86_64-linux
  `for_range` residual — confirmed on the first hypothesis test (reverting
  to `Vec<Handle>` reversed the mark every time), so the ladder never
  needed a second hypothesis in practice.
- **The footer incident.** How "record decisions with their scope" got
  its name: a narrow decision ("trailers accepted in commits") was later
  remembered as a broader one ("footers accepted"), which is how the
  eventual triple-footer problem started.
- **The 90-PR retroactive sweep (2026-07-18).** The model for what the
  four-step codification act's step (iv) — an immediate consistency audit
  against a newly-codified rule — looks like at scale. The standing-watch
  class had been codified without running step (iv), which left other
  negative-results entries unexamined; step (iv), run late, found two
  whose stated premise (the dispatch codegen lottery) H1 had since killed.

## Native graphics & compute rollout timeline

The standing roadmap directive in `CLAUDE.md` describes what's still
open (GL-compute) and the settled sequencing/SPIR-V decisions. This is
the historical account of how the rest of it actually landed.

Sequencing as it happened: the Metal arc first (graphics phases, then
Metal compute reusing the same device/queue machinery), then Vulkan
(compute, then graphics), then OpenCL, CUDA, and DirectX — shared cores
extracted at each second-of-its-kind backend, not guessed up front.

The `wgpu`/`pollster` dependency (v0.7's one optional dependency, behind
a `gpu` feature) was deleted the same day the coverage condition was
met: 2026-07-17, once Metal, Vulkan (compute + graphics), and OpenCL
(with a CI-proven real dispatch) were all in. CUDA compute (`src/cu.rs`),
DirectX (`src/dx.rs`), and the Win32 Vulkan window surface
(`src/window/win32/vulkan.rs`) shipped afterward, all in v0.8. Every
build of Socrates has been zero-dependency since that day.

## Session-mechanics incidents

The rules in `CLAUDE.md`'s "Session mechanics" list are stated as plain
directives; each of the following is the incident that produced one.

- **Wrong-checkout commits.** More than one recorded incident of a
  git-mutating command running against the wrong local checkout — the
  reason every such command now opens with
  `cd <dir> && pwd && git branch --show-current`.
- **Multi-clone confusion.** Post-rename re-registration is how a
  session first ended up holding more than one clone of the repo at
  once, motivating the "pull the harness-served clone after every
  CLAUDE.md-touching merge" rule.
- **The 2026-07-18 bulk branch cleanup** is the precedent for "branches
  are deleted only in user-directed cleanups, never unilaterally," and
  for moving anything a standing record references to `archive/*`
  first.
- **The whole-tree `cargo fmt` measurement (2026-07-18):** running
  `cargo fmt --check | grep -c '^Diff in'` on this tree found hundreds
  of hunks across the whole codebase, confirming the tree has never
  been through a bare `cargo fmt` — the number drifts as the tree
  grows, which is why the rule says to re-measure rather than trust a
  fixed figure.
- **The "Claude Fable 5" commit-trailer leak.** A model-identifier
  variant leaked into every commit trailer for a full session before
  being caught (2026-07-18) — the existing "no model identifier in any
  pushed artifact" wording didn't name the commit-trailer channel
  explicitly, so it wasn't checked against a rule that already covered
  it in principle. Fixed prospectively; past commits were not rewritten.
- **The PR #103 x86_64-linux investigation's agent stalling twice in a
  row** — reporting "standing by" and ending its turn instead of
  actually waiting on a CI run — is what produced the "a delegated
  subagent polls within its own turn, not by ending it" rule (the
  mirror image of the main session's own scheduled-check-in mechanism,
  which the subagent doesn't have access to).
- **The `get_status`-returns-empty incident (2026-07-18).** This repo
  reports exclusively through the newer Checks API; a combined-status
  API call returned zero checks, which got misread as "still pending"
  instead of a wrong-tool warning sign — two PRs sat fully green while a
  scheduled check-in was awaited instead of a direct, correct-API look.
  Produced the "a signal is a prompt to check, not a substitute for
  checking" rule.
- **The missed-reschedule incident (2026-07-18).** A check-in armed for
  in-flight bench-matrix samples went quiet with the work unfinished and
  no follow-up wakeup armed — the user had to notice the stall and ask.
  Produced the "a wakeup firing is never a terminal, silent event" rule.
- **The `CLAUDE.local.md` → `CLAUDE.md` saga (PR #107, 2026-07-19).**
  Roxy noticed Claude Desktop's context-tracker "Memory files" panel
  only ever listed root `CLAUDE.md`, even though `docs/SPEC.md`,
  `bench/RESULTS.md`, and friends were already the project's detailed
  memory. The fix took three rounds of correction in one session:
  1. First pass nested stub files at `<dir>/.claude/CLAUDE.md` — not a
     real discovery path; `.claude/` inside a subdirectory is reserved
     for settings/skills/rules, confirmed against Claude Code's monorepo
     docs. Nested memory discovery only looks for a bare
     `<dir>/CLAUDE.md`.
  2. Second pass caught a real cost: the `@`-import inside such a stub
     isn't lazy about *content* — the moment any file in that
     subdirectory is read, the stub force-loads the entire imported
     file(s) into every session's context, for every contributor,
     indefinitely.
  3. Given that cost, the mechanism landed first on `CLAUDE.local.md`
     (gitignored, personal, opt-in per checkout) rather than committed
     `CLAUDE.md` — deliberately the lower-risk first step: prove the
     mechanism before committing it to the shared repo.
  Once it had run clean for a session, Roxy's follow-up directive
  flipped all four stubs from `CLAUDE.local.md` to committed `CLAUDE.md`
  and dropped the now-unneeded `.gitignore` rule. The exact stub content
  at that point (kept here for the record, though the files themselves
  are now the live source of truth):

  | File | Content |
  | --- | --- |
  | `docs/CLAUDE.md` | `@SPEC.md` / `@ARCHITECTURE.md` / `@RELEASING-macOS.md` |
  | `bench/CLAUDE.md` | `@RESULTS.md` |
  | `demos/CLAUDE.md` | `@NOTES.md` / `@STYLE.md` |
  | `ports/CLAUDE.md` | `@README.md` / `@pyl/CONTRACT.md` / `@icaa/README.md` / `@claudewave/README.md` |

## Consistency and workflow incidents

- **The spec-count drift release.** A release draft once shipped saying
  311 golden spec tests while the suite actually stood at 313 — the
  reason `tools/check_counts.sh` exists and why the spec-suite count is
  cross-checked in all five of its stating places on every CI run.
- **The 2026-07-18 macos-14 DNS incident.** The first instance of "a
  fixed target does not rot": a macos-14 CI job's runner had its DNS
  fail resolving `codeload` mid-job, after fetching from the same host
  seconds earlier — an infrastructure blip, not a reason to distrust the
  pinned runner image itself.
- **`archive/h2-small-list` and the W1a hold** are the precedents for
  "non-landing work is pushed for durability without a PR": a dropped
  probe or a held wave lives on its own pushed branch rather than being
  discarded or forced into a PR that was never going to merge.
