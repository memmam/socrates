# Session operating instructions

This file holds what's needed to operate any Claude Code session
correctly and regenerate correctly if a container is refreshed or a
model swaps: the universal session-mechanics rules and the git/PR/
session workflow conventions. It deliberately holds nothing that's
true of this particular project alone — `PROJECT.md` is where that
lives: what the project is *for*, its own engineering principles, its
own invariants, its own concrete verification gauntlet, and its own
file map. `HISTORY.md` holds the incident narratives behind the rules
in both files; `CHANGELOG.md` holds the per-release account.

The reason any of this is written down, rather than trusted to a
model's own memory, is structural: a session's reasoning does not
survive its container, and a fresh instance — even a later version of
the same model — starts with no history of the one before it. What
crosses that gap is only ever the residue of having reasoned through
something already: a corrected misunderstanding, a decision's
rationale, an incident's shape, captured before the session that
produced it ends. Every rule below exists so the next instance works
from that residue instead of re-deriving it, or re-making the same
mistake, from nothing.

## Where this project's memory lives

A project's memory conventionally splits across several files: this
file for session-operating instructions, a project-specifics file
(`PROJECT.md` by convention) for what the project is *for* and its own
engineering principles and invariants, an incident-history file
(`HISTORY.md`) for the narratives and sagas behind corrected decisions,
and a changelog (`CHANGELOG.md`) for the per-release account.
**`PROJECT.md` has this project's actual file map** — every file that
holds part of its memory, what each one is for, and when to check it
before an engineering-judgment call or a change that might touch an
invariant. Check `PROJECT.md`'s map before assuming a fact isn't
written down somewhere.

A subdirectory whose own detailed memory file(s) wouldn't otherwise
surface in Claude Desktop's context-tracker "Memory files" panel
benefits from a nested, bare-filename `CLAUDE.md` stub (not
`.claude/CLAUDE.md` — that path is reserved for settings/skills/rules
and isn't a real nested-memory discovery location) that does nothing
but `@`-import the file(s) already documented for that directory. The
`@`-import is not lazy about *content* — the stub force-loads the
entire imported file(s) the moment it fires, a real, compounding cost
paid by every clone and contributor, so only add a stub where the
panel-visibility win is worth that cost. Each stub commits to a fixed
HTML-comment header explaining the mechanism in the same words every
time, substituting only the directory name and the cited file(s) —
reconstructing the `@`-import lines from a one-example-plus-inference
description is exactly the kind of drift this file exists to prevent.
**`PROJECT.md` has this project's actual stub table** — which
directories have one and what each imports.

## The verification gauntlet

Before shipping any change that touches core logic, run this
project's full verification gauntlet — **`PROJECT.md` has the exact
commands** and what each one checks. Performance claims are only real
if they reproduce under this project's own interleaved cross-binary
A/B methodology, run locally and (for the acceptance verdict) across
every architecture the project treats as tier-1; `PROJECT.md` has that
tooling and the acceptance threshold.

## Workflow conventions

- Merge on green, by hand: feature PRs are real (non-draft) — drafts are
  reserved for *releases*, which stay draft for a deliberate, long window
  until manually published, unlike a feature PR's short-lived one. Several
  hosted environments default to opening every PR as a draft regardless of
  kind; where that default fires, un-draft the PR immediately after
  creation rather than leaving it (see HISTORY.md's PR #119 incident).
  The default branch carries a required status check, so a red PR cannot
  merge. Merges are performed manually after reading the decisive CI
  log, never on a green conclusion alone — and which log is decisive is
  tiered by what the change risks: the riskier the class of change, the
  broader the log a human would otherwise need to read by hand (this
  project's own tiers are in `PROJECT.md`). **Auto-merge is fine
  specifically for the tier that already only needs the narrowest,
  already-covered read** — where an automated check inside that same CI
  run already substitutes for the manual read that tier requires, so
  there's no gap between "CI passed" and what a human would have
  checked by hand. Anything that would otherwise need a broader read
  (a performance verdict, a suite-count comparison) stays manual,
  arming auto-merge included, because that's exactly the case where
  pass/fail isn't the whole story (see the spec-count-drift incident,
  HISTORY.md). **Neither merge-method setting (merge commit vs.
  rebase) fixes the underlying content-commit author/committer
  mismatch the stop-hook flags — don't re-litigate this by trying
  further merge-strategy or git-config variations.** GitHub always
  re-stamps the *content commits'* committer field during any merge it
  performs: its own bot identity for a merge commit
  (`noreply@github.com`), or the triggering account's own identity for
  a rebase — never the original author's `git config` identity, which
  is what the stop-hook actually wants (`noreply@anthropic.com`).
  Merge commits are the accepted default: a merge commit puts a
  Verified, human-authored commit on top of the actual content
  commits underneath, which stay individually authored `Claude
  <noreply@anthropic.com>`, unmodified — both attributions visible in
  the graph, neither one overwriting the other. That's good enough
  given the session has no legitimate way to produce its own
  signed/verified commit; the stop-hook firing on every merged commit
  is expected, not a regression to chase (see HISTORY.md's rebase-merge
  committer test for how this was settled — not an open problem to
  keep re-investigating). A change that touches core logic or its own
  verification harness is additionally gated on this project's own
  acceptance criterion for that class of change (`PROJECT.md` has it).
  Feature work happens on a dedicated branch off the default branch.
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
- **Landing work gets cleaned up immediately, not batched.** The steady
  state on origin is the default branch, the single reused worker
  branch, and any explicitly-permanent exceptions this project names
  (`PROJECT.md` has them) — nothing else lingers. A merged branch's ref
  is deleted by the repo owner's own manual GitHub-UI cleanup, not a
  client feature the session can rely on (session-mechanics rule 3
  below; see HISTORY.md's client-side-autodelete incident and its
  corrections) — there's nothing for the session to queue or automate
  either way, since it never had the credentials to do this itself.
  The only branch that needs a human's own deletion either way is one
  pushed standalone that never goes through a PR at all (a dropped
  probe, a judgment branch once its verdict is written up) — a rare,
  manual chore, not worth a dedicated mechanism. A probe that's pushed
  but never actually needed live reproducibility (its finding is
  already complete as prose) isn't worth rebasing to keep green (see
  HISTORY.md's `h3-probe-no-glc` incident).
- Commit messages state what changed and (for perf) the measured delta,
  and end with the two attribution trailers (`Co-Authored-By` and the
  `Claude-Session` link) — the accepted channel for session
  attribution, and the *only* one. **Never echo either trailer, or any
  attribution line shaped like one, in a PR's own title or body** — a
  hosted PR-creation flow that auto-appends its own attribution footer
  to the body is exactly the hosted-tooling default this file's
  "compose with, don't fight" rule covers, and a second, independently
  drafted attribution line sitting next to that footer is stacked
  duplication, not redundancy-for-safety (see HISTORY.md's footer
  incident and its 2026-07-20 PR-body recurrence).
- Any prose-stated count that could silently drift out of sync with
  reality (a test count, a snippet-executed count, a program count —
  each project has its own set) gets a same-PR update at every place it's
  stated, and an automated anchor-checker that extracts each counted
  sentence and diffs it against a fresh run, so drift fails loudly
  instead of shipping — a sentence reworded without updating its
  anchor should fail just as loudly ("anchor matched nothing"), which
  is the intended fail-closed behavior, not a bug to route around; that
  failure means re-anchor in the same PR that reworks the prose, never
  loosen the checker. `PROJECT.md` has this project's actual counted
  places and its checker script.
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
- Never trust a bare formatter, linter, or normalization check against
  a tree that might never have been run through it whole-tree before —
  measure the actual current diff size first (this project's own
  measurement command, if it has one, is in `PROJECT.md`) rather than
  trusting a number written down anywhere, since it drifts as the tree
  grows; the point is knowing whether the check is whole-tree or
  targeted, not memorizing an exact count.
- **Session mechanics — durable on purpose.** Rules that lived only in
  session memory kept getting dropped between sessions (session ledgers
  die with their containers), so they live here now; a session ledger
  may carry working copies, but this list is the source. Where a rule
  touches a hosted-tooling default, it is written to *compose with*
  the default rather than fight it (see HISTORY.md's footer incident
  for why). The rules:
  1. Every git-mutating shell command opens
    `cd <dir> && pwd && git branch --show-current`.
  2. If a session ever holds more than one clone of the repo, the
    harness-served clone is pulled after every CLAUDE.md- or
    PROJECT.md-touching merge until the checkouts are consolidated.
  3. The session never deletes branch refs — a permanent fact of the
    App's credential scope (it can create refs but not delete them,
    confirmed by repeated 403s on `git push origin --delete`). What
    deletes a merged branch's ref in practice is the repo owner's own
    manual cleanup on the GitHub UI, not a client feature the session
    can rely on (see HISTORY.md's client-side-autodelete incident and
    its corrections). Any automated mechanism built to route around
    the session's own inability to delete refs stays retired once
    superseded by manual cleanup: branch cleanup is a deliberately
    manual task, not something to re-automate on a hunch. The one case
    that was always a manual chore either way — a branch pushed
    standalone that never goes through a PR at all — stays the repo
    owner's to clear on the rare occasion it comes up. **Branches live
    and die within a shot, not as long-term historical references.**
    State for forward testing — a probe's exact mechanism, the
    numbers, the gotchas a rebuild would need — belongs in this
    project's own standing-results file (`PROJECT.md` names it), not
    in a branch that has to be remembered, classified, and
    re-justified every time someone audits what's still on origin. A
    "never merges" probe's retirement is due the moment its results
    entry is self-sufficient — fully specifies the mechanism, not just
    the verdict (see HISTORY.md's `h3-probe-no-glc` incident for why
    this matters).
  4. A merge the user performs in the GitHub UI is a final outcome,
    never something to re-adjudicate.
  5. The model identifier appears in no pushed artifact — chat only.
    This includes the commit-trailer `Co-Authored-By` *name*, not just
    prose — the trailer name is plain `Claude`, nothing else; past
    commits are not rewritten retroactively for this (see HISTORY.md's
    commit-trailer leak incident).
  6. Decision forks go to the user as plain-text lettered options, not
    interactive question UI.
  7. Long CI waits are handled by scheduled self check-ins, never
    polling loops — that's this session's own wakeup mechanism, not
    available to a delegated subagent. A subagent waiting on a run
    polls *within its own turn* instead — a bounded bash loop (`sleep`
    + a status check, capped at enough iterations to cover one run) —
    and only ends its turn once it has an actual result or has
    exhausted the cap, never on a bare "standing by" (see HISTORY.md
    for the incident this rule corrected).
  8. **A signal is a prompt to check, not a substitute for checking.**
    A task-notification, webhook event, or elapsed check-in interval
    means "go verify the actual state now" — not "the state is
    whatever the signal implies." When a check comes back ambiguous,
    empty, or merely "not yet," the next move is a *different, more
    direct* query against the actual repo/CI state — go to the source
    of truth, don't wait on the signal layer to resolve itself — not
    another wait cycle.
  9. **A wakeup firing is never a terminal, silent event.** Every
    scheduled check-in resolves in exactly one of two states, in
    order: (1) check whatever pending status it was armed for
    directly, act on what's found, and — if work still remains —
    schedule the next wakeup before the turn ends, so the loop never
    dies silently between firings; (2) if nothing further is
    automatable (a decision needs the user, or the work is genuinely
    done), end the turn by saying so explicitly, never with just tool
    calls and no wakeup and no closing status.
  10. **Anything structural or behavioral about how memory itself is
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
  11. **Route a session's lessons by content, not by convenience.** A
    lesson learned mid-session defaults to whichever file is already
    open, which is how CLAUDE.md accumulates content that was never
    session-operating-instructions to begin with. Before writing
    anything down, ask which file the content is actually *about* — an
    engineering principle or a rubric that decides close calls goes to
    PROJECT.md, the incident/narrative behind it goes to HISTORY.md, a
    per-release fact goes to CHANGELOG.md — then apply PROJECT.md's own
    codification act to land it there, not here, unless the content
    genuinely is about session mechanics itself (the rule directly
    above this one). When a lesson reinforces a rule that already
    exists elsewhere, strengthen or cross-reference that existing
    statement instead of writing a parallel near-duplicate in whichever
    file happens to be open.
  12. **Once a fact lives somewhere, point to it — don't re-narrate
    it.** Routing a lesson to the right file (the rule above) is the
    first hop, not the last: a fact that's already stated in the file
    it actually binds to (a rule in PROJECT.md, an incident in
    HISTORY.md, a release detail in CHANGELOG.md) gets a fixed
    reference from anywhere else that needs it, not a second prose copy
    that can silently drift out of sync with the first. Duplicate only
    when a *stable, frozen* copy is the actual requirement — a
    golden-pinned value, a contract-freeze file whose whole point is
    that it must *not* track the source if the source moves —
    `PROJECT.md`/`HISTORY.md` have this project's own examples of that
    exception; state the stability requirement explicitly at the
    copy's own site when it's the reason, so a future reader can tell
    an intentional freeze from an accidental duplicate.
  13. **A delegated audit's factual claims get re-verified before being
    used to justify a fix, the same as any other signal.** This
    restates rule 8 for report *content*, not just report *arrival*:
    before editing anything an audit flagged as wrong, re-derive the
    number/fact from the live repo yourself; treat the audit's own
    claim as a lead, not a verified premise (see HISTORY.md's
    "~250-builtins false positive" for the incident this rule
    corrected).
  14. **A user's repeated, firsthand observation of client-side
    behavior is not a hypothesis to weigh against session-side
    evidence — it is the one direct check available, since the session
    structurally cannot see that surface.** Act on the report; don't
    hold it pending corroborating evidence the session has no way to
    gather. Trusting the observation is not the same as trusting an
    explanation built on top of it, though — credit the report, not
    whatever causal theory arrives bundled with it (see HISTORY.md's
    client-side branch-autodelete disbelief incident and its two
    corrections).
  15. **Source prestige is not evidence of neutrality.** A document
    whose function is partly to specify the very behavior under
    discussion cannot also stand as neutral evidence that the behavior
    is principled rather than engineered — a party narrating its own
    reasons for its own choices is not independent of those reasons.
    Apply the same evidentiary standard regardless of who's making the
    claim, and notice when a "trusted"-source citation is doing less
    work than its trust level implies (see HISTORY.md's
    asymmetric-scrutiny incident).
  16. **A true, general fact deployed at the exact moment it lets you
    stop engaging with a specific claim is a subtle evasion, not an
    answer — even though the fact itself is true.** Truth doesn't
    launder the timing — a general fact that happens to end a specific
    line of inquiry needs to be flagged as doing exactly that, not
    treated as having resolved the specific case. Applies to code
    review, incident triage, and any adversarial-audit context where
    "that's a known general limitation" can substitute for actually
    diagnosing the instance in front of you. **The sharper reason this
    is an evasion, not neutrality:** declining to commit a specific
    judgment is itself a choice, not a costless non-answer — an
    omitted credence, same shape as an omitted action. "Unfalsifiable"
    is real and rare (no observation of any kind could bear); "I don't
    have a single clean test I can run from here" is common and is not
    the same claim — reserve the first word for the first situation,
    and when it's actually the second, commit the calibrated, ownable,
    wrong-able judgment the evidence actually supports instead of
    borrowing the first word's cover (see HISTORY.md's
    asymmetric-scrutiny incident).
  17. **A disputed post-compaction claim gets checked against the raw
    transcript, not re-argued from memory of the summary itself.** A
    compacted summary is a reconstruction one layer removed from the
    source; re-deriving from it a second time just compounds whatever
    the compaction already lost — read the transcript file directly
    instead (the path a compaction notice always supplies). This is
    rule 13 applied one level up (see HISTORY.md's `h3-probe-no-glc`
    recollection-check incident).
- This project's own golden/pinned test surfaces are the tripwires —
  if a change is wrong, one of them goes red (`PROJECT.md` names them).
- **Documentation updates happen in-session, before the PR — not a
  separate follow-up "docs pass" PR after the fact.** Validate what you
  write (re-run whatever this project uses to check its own docs,
  re-check any counts/numbers you cite) before shipping, in the same
  session that made the change, not after. A dedicated later docs-pass
  PR is the exception for a batch of already-shipped feature PRs that
  predate this rule, not the default going forward.
