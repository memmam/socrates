# Socrates — session operating instructions

Socrates is a zero-dependency Rust interpreter for an AI-native
programming language. This file holds only what's needed to operate a
session correctly and regenerate correctly if a container is refreshed or
a model swaps: where the project's memory lives, the verification
gauntlet, and the git/PR/session workflow conventions. `PROJECT.md` holds
what Socrates is *for*, the engineering principles that decide close
calls, the native graphics/compute roadmap, and the invariants that must
never break — check it before any engineering-judgment call, and before
touching anything the invariants guard. `HISTORY.md` holds the incident
narratives behind the rules in both files; `CHANGELOG.md` holds the
per-release account.

## Where the project's memory lives

- `PROJECT.md` — what Socrates is for, the engineering principles, the
  native graphics/compute roadmap, and the invariants. Check it before an
  engineering-judgment call, before a change that might touch an
  invariant, or before touching the graphics/compute roadmap.
- `HISTORY.md` — the incident narratives and sagas behind the rules and
  corrected decisions in this file and PROJECT.md. Check it when a rule
  points here, or when auditing whether a rule still matches the
  incident that produced it.
- `CHANGELOG.md` — the per-release account: feature lists, benchmark
  deltas, and mechanism detail (each release shipped as one PR). Check
  it for release-post material or the full story behind any rename or
  shipped feature a rule only mentions in passing.
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
  process/history belongs in `CLAUDE.md`, `PROJECT.md`, `HISTORY.md`, or
  `CHANGELOG.md`, never in the book).

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
./target/release/socrates test demos/!(glcube)/ demos/glcube/cube.soc demos/glcube/spec.soc  # 68, also with SOCRATES_GC_STRESS=1
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
acceptance verdict, per PROJECT.md's
universality principle: flat-or-better on every tier-1 architecture.
`bench/run.sh [N]` is single-binary sequential profiling convenience
(where does one binary spend its time?), not the gate. Method and standing
numbers: `bench/RESULTS.md`.

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
  matrix verdict (PROJECT.md has the acceptance criterion: flat-or-better
  on every architecture) — `RESULTS.md` prose is exempt (it changes no
  binary; its matrix run would be an A/A that tests nothing). The verdict
  attaches to the tree that built the judged binaries: follow-up
  commits that touch no compiled source (docs, prose) ride the
  existing verdict without a re-run. Feature work happens on a
  dedicated branch off `main`.
- **When a PR's scope expands past what its own description promised —
  a "being worked in a follow-up" item gets folded into this PR
  instead — post a PR comment saying so explicitly.** The description
  is a claim, same as any other prose in the tree, and it goes stale
  exactly the way any claim does: silently, if nothing corrects it. New
  commits speak for what changed, never for what the description now
  gets wrong about scope — a reader skimming the description alone
  should not be misled about what's actually in the PR. Update the
  description too when the stale line is a simple, one-line drift-fix
  (e.g., "Tiers 2-4 are a follow-up" once they no longer are); the
  comment is the part that doesn't get skipped, since it lands in the
  PR's timeline where following eyes actually look, rather than
  depending on someone re-reading a body that already looked settled.
- Non-landing work is pushed for durability without a PR: a dropped
  probe or a held wave lives on its own pushed branch rather than being
  discarded or forced into a PR that was never going to merge. PRs are
  for changes meant to merge, drafts for releases, archival branches for
  neither.
- Commit messages state what changed and (for perf) the measured delta,
  and end with the two attribution trailers (`Co-Authored-By` and the
  `Claude-Session` link) — the accepted channel for session
  attribution, and the *only* one.
- The spec-suite count is stated in exactly six places — `README.md`
  (×2), `CLAUDE.md` (×1, the gauntlet), `PROJECT.md` (×1, the
  invariants), `.github/RELEASE_NOTES.md` (×1), and
  `book/11-toolchain.md` (×1) — and a count change updates all six in
  the same PR. The same discipline covers every other prose-stated
  count — book snippets executed/total, the demo-golden count, the
  spelled-out demo-program count — each with its own set of stating
  places. `tools/check_counts.sh` (run in CI's Test
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
    harness-served clone is pulled after every CLAUDE.md- or
    PROJECT.md-touching merge until the checkouts are consolidated.
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
    configured** — CLAUDE.md/PROJECT.md/HISTORY.md conventions, what
    gets committed, nested-stub patterns and their exact content —
    lands here, byte-exact, the same session it's decided, same as any
    other rule on this list. This is that rule applied to itself:
    Claude Code's own auto-memory ("memories folder",
    `~/.claude/projects/<project>/memory/`) is explicitly machine-local
    and does not survive a fresh container or a different session's
    checkout, and a scratchpad session ledger is even more ephemeral
    (gone the moment its container is reclaimed) — either one is a
    *reconstruction* source, not a durable one, and reconstructing from
    a partial or ambiguous description (one example generalized by
    inference, say) is where drift creeps in between sessions or across
    a model swap.
  - **Route a session's lessons by content, not by convenience.** A
    lesson learned mid-session defaults to whichever file is already
    open, which is how CLAUDE.md accumulates content that was never
    session-operating-instructions to begin with. Before writing
    anything down, ask which of the files in "Where the project's
    memory lives" the content is actually *about* — an engineering
    principle or a rubric that decides close calls goes to PROJECT.md,
    the incident/narrative behind it goes to HISTORY.md, a per-release
    fact goes to CHANGELOG.md, a demo house rule goes to demos/STYLE.md
    (or its ledger in demos/NOTES.md) — then apply PROJECT.md's own
    four-step codification act to land it there, not here, unless the
    content genuinely is about session mechanics itself (the rule
    directly above this one). When a lesson reinforces a rule that
    already exists elsewhere, strengthen or cross-reference that
    existing statement instead of writing a parallel near-duplicate in
    whichever file happens to be open.
  - **Once a fact lives somewhere, point to it — don't re-narrate it.**
    Routing a lesson to the right file (the rule above) is the first
    hop, not the last: a fact that's already stated in the file it
    actually binds to (a rule in PROJECT.md, an incident in HISTORY.md,
    a release detail in CHANGELOG.md, a spec detail in `docs/SPEC.md`)
    gets a fixed reference from anywhere else that needs it, not a
    second prose copy that can silently drift out of sync with the
    first — HISTORY.md's own "Release ledger" section (pointing at
    CHANGELOG.md instead of restating the release-by-release account it
    used to carry in full) is the standing model. Duplicate only when a
    *stable, frozen* copy is the actual requirement — a golden-pinned
    value, a benchmark's `// Bench:` epoch bridge, a contract freeze
    file like `ports/pyl/CONTRACT.md`'s `sos_freeze.txt` — where the
    whole point of the copy is that it must *not* track the source if
    the source moves; state that stability requirement explicitly at
    the copy's own site when it's the reason, so a future reader can
    tell an intentional freeze from an accidental duplicate.
- The spec, the book's executable snippets, and the demos' pinned output are
  the three tripwires — if a change is wrong, one of them goes red.
- **CHANGELOG, book, README, and ARCHITECTURE updates happen in-session,
  before the PR — not a separate follow-up "docs pass" PR after the fact.**
  This matches `docs/SPEC.md`'s own same-PR rule (see PROJECT.md's
  Invariants section) and now extends to the rest of the documentation
  set: validate what you write (run the book-snippet suite, re-check any
  counts/numbers you cite) before shipping, in the same session that made
  the change, not after. A dedicated later docs-pass PR is the exception
  for a batch of already-shipped feature PRs that predate this rule, not
  the default going forward.
