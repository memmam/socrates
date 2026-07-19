# Socrates — project history

`CLAUDE.md` (session-operating instructions) and `PROJECT.md` (what
Socrates is for, its engineering principles, and its invariants) are
kept lean on purpose: what's needed to operate a session, follow the
engineering principles as meant, and regenerate correctly if a
container is refreshed or a model swaps. This file is where the
*historical* half of that content lives instead — the incidents that
motivated a rule and the sagas behind a corrected decision. Nothing
here is required to operate correctly today; it's the evidence trail
for anyone who wants to know why a rule reads the way it does. The
per-release account itself — feature lists, benchmark deltas, the
concrete mechanism detail — lives in `CHANGELOG.md`, not here.

Read `CLAUDE.md` and `PROJECT.md` first. Come here when a rule's own
text points here, or when auditing whether a rule still matches the
incident that produced it — or `CHANGELOG.md` when what you need is
the per-release account instead.

## The rename: Fable → Socrates

Recorded 2026-07-18. The full rationale — trademark pre-emption, the
candidate names considered ("Timaeus" reserved for the eventual
top-of-stack agent, "Quine" rejected since an existing OSS graph
database holds it), and the `.soc` extension's nod to the HDL
roadmap's system-on-a-chip trajectory — is in `CHANGELOG.md`'s v0.8.0
entry ("Renamed"), alongside the concrete rename touchpoints (binary
and package name, env vars, the Mach-O payload section) that aren't
repeated here. The one fact that's operationally relevant today —
`bench/ab.py` and the Bench A/B workflow's permanent cross-name
fallback, keeping pre-rename refs benchable — is stated in `PROJECT.md`
itself.

## Release ledger

The v0.1–v0.8 release-by-release account used to live here in full.
Once `CHANGELOG.md` carried the richer per-release account for every
one of those releases — feature lists, benchmark deltas, the concrete
mechanism detail, all of it exceeding what was written here — keeping
a second, terser copy in this file just duplicated it without adding
anything, so this section now points there instead of restating it:
`CHANGELOG.md` is the source material for release posts going forward,
not this list.

## Engineering-principle incidents

The rules these motivated are stated plainly in `PROJECT.md`; this is the
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
- **The `std.wav` decode-trim (2026-07-19).** Roxy's question — is
  `std.wav` minified as much as possible, "the whole point is that these
  need to be minimum implementations" — caught a `decode()` whose only
  caller was its own round-trip test. Cutting it (and the mirrored
  `pyl.audio.read_wav`) is the first instance of "`std` surface is earned,
  never speculative" as a project-wide principle, not just a demos-local
  one. The immediate follow-up directive — promote any demo that's
  strictly a file generator into `std`, splitting for atomic reusability —
  produced the six-module wave (`std.wav`/`svg`/`markdown`/`crc`/`zlib`/
  `png`) the same session, verified byte-identical against every existing
  golden across all five affected demos.
- **The `deflate_stored`/`inflate_stored` → `wrap`/`unwrap` rename
  (2026-07-19).** During the same promotion wave, Roxy's naming
  constraint — "if it's not LITERALLY DEFLATE, rename it to what it
  ACTUALLY is" — caught that `demos/png/zlib.soc`'s functions only ever
  emit RFC 1951's uncompressed *stored* block type, never real
  LZ77/Huffman compression, so the `deflate`/`inflate` names overclaimed.
  Renamed to `wrap`/`unwrap` (and `Inflated` to `Unwrapped`), rationale
  recorded in the module's own header comment — the first instance of "a
  name is a claim" as a project-wide principle. No forward instance
  exists yet (nothing has been deliberately changed out from under an
  existing name since) — the forward half of the rule (rename before
  drifting further, and give spec-correctness-dependent callers their
  own real implementation rather than an evolving name) is preventive,
  recorded ahead of its first occasion on purpose.

## Native graphics & compute rollout timeline

The standing roadmap directive in `PROJECT.md` describes what's still
open (GL-compute) and the settled sequencing/SPIR-V decisions; it
landed exactly as sequenced there (Metal, then Vulkan, then
OpenCL/CUDA/DirectX — see PROJECT.md's own "Sequencing" bullet rather
than restating it here). What PROJECT.md's roadmap doesn't date is
when the coverage condition was actually met:

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
  CLAUDE.md- or PROJECT.md-touching merge" rule.
- **The 2026-07-18 bulk branch cleanup** is the precedent for "branches
  are deleted only in user-directed cleanups, never unilaterally," and
  for moving anything a standing record references to `archive/*`
  first.
- **The 2026-07-19 cleanup and the weekly sweep it produced.** Roxy
  asked for a cleanup and for cleanups to become automated going
  forward. Nineteen merged branches (PR #116) were the first instance
  of the merged-only test — `git merge-base --is-ancestor` before
  listing, so the `archive/*` step never applies (main already carries
  a merged branch's content) — while four unmerged branches
  (`bench/h1-binding-recheck`, `bench/inline-upvals`,
  `bench/inline-upvals-x64-probe`, `probe-cmp-branch`) were left for a
  human, matching "non-landing work stays pushed." That merged/unmerged
  split, not "is it referenced," is what a weekly Routine can safely
  run unattended — proposing the PR, never merging it.
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
- **The codification-routing gap (2026-07-19).** Roxy's directive to
  codify anything from a session that needs codifying, and make sure
  it's "distributed to the right files rather than just living in
  CLAUDE.md," surfaced that this same session's own std-minimality and
  naming-accuracy lessons had landed only in `CHANGELOG.md`/
  `ARCHITECTURE.md`'s per-release prose — no engineering-principle
  statement in `PROJECT.md`, no incident trail here. A real gap, not a
  hypothetical one. Produced the "route by content, not by convenience"
  rule; this entry is its own first instance, the rule that motivated
  writing it down being the one now on record.
- **The redundancy-cascade follow-up (2026-07-19).** Roxy's check
  immediately after the entry above asked whether the routing rule went
  far enough — whether facts, once routed to PROJECT.md/HISTORY.md,
  then cascade correctly into CHANGELOG.md and the actual docs, with
  redundancy replaced by fixed references except where a frozen copy is
  the real requirement. It hadn't: only the first hop (session lesson →
  which file) had been written down, not the second (established fact →
  point, don't re-narrate). Produced "once a fact lives somewhere,
  point to it," grounded in the Release ledger section's own existing
  practice rather than a new invented example.

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
- **PR #115's silent scope creep (2026-07-19).** Its own description
  said "Tiers 2-4 from the audit are being worked in a follow-up," then
  Tier 2, Tier 3, and Tier 4 all landed as further commits on this same
  PR instead — with nothing said about it anywhere but the commit log.
  Roxy caught it mid-merge-attempt: the added commits had been left to
  speak for themselves, and the description sat there actively wrong.
  Produced "when a PR's scope expands past what its description
  promised, say so in a comment" — the description is a claim like any
  other, and claims go stale silently unless something corrects them.
