# Benchmark results

The measurement instrument is `bench/ab.py BASE_DIR HEAD_DIR` — an
interleaved cross-binary A/B between two full checkouts — fanned across
one runner per tier-1 architecture by the Bench A/B workflow
(`.github/workflows/bench.yml`), which fires on pushing a `bench/<name>`
branch — the official re-sampling path would be re-dispatching the
workflow or re-running a job, but the bot account's API calls to
workflow_dispatch and re-run both return 403, so extra samples are
obtained by pushing empty commits to the bench branch instead; re-check
this workaround whenever the App's permission scope changes (the same
403 boundary CLAUDE.md's session-mechanics rule 3 records, for ref
deletion specifically).
`bench/run.sh [N]` is a single-binary sequential profiling convenience
— where does one binary spend its time? — not the gate.
Micro-benchmarks (`bench/*.soc`) isolate one cost centre each,
stated in each file's `// Bench:` measurand header; macros are the heavy
demo mains. The spec suite appears only in run.sh's single-tree rows,
never as an A/B target (its sources move with each ref — see below).

Every bench program prints a checksum, and ab.py enforces it: every rep
of a (target, side) pair must produce byte-identical stdout (a mismatch
is a hard failure naming the target and side), and when a target's
sources are byte-identical between the two trees, base stdout must equal
head stdout — a wrong-answer "optimization" fails the run instead of
winning it. When sources legitimately differ between refs, only the
per-rep stability check applies.

**Method for any perf claim.** Build a release binary of the change and a
release binary of the pre-change tree, then A/B them *interleaved*
(alternate binaries within one batch, best-of-4+), on a quiet box. Layout
and machine-state noise on a shared box swings single-shot numbers ±5–10%;
a claimed win must beat that noise or be backed by instruction/allocation
counts. Absolute times drift between machines and over a day — trust the
relative delta, not the absolute seconds.

**And across architectures.** One box is one vote: code layout,
indirect-branch cost, and cache geometry vote differently per
architecture, so a simplification that measures flat on x86_64 can
regress on aarch64 (and vice versa). `bench/ab.py BASE_DIR HEAD_DIR`
runs the interleaved A/B between two full checkouts — each side runs its
own tree, so the comparison stays fair when bench/demo sources differ
between the refs (this is also why the spec suite is not an A/B target:
its sources move with each ref). The **Bench A/B workflow**
(`.github/workflows/bench.yml`, fired by pushing the candidate as a
`bench/<name>` branch)
fans the same script across one runner per tier-1 architecture —
x86_64-linux, aarch64-linux, x86_64-windows, aarch64-macos — and posts
each delta table to the run summary. The acceptance rule is PROJECT.md's
universality principle: flat-or-better everywhere, or the idiom keeps
its primitive.

The `bench/<name>` namespace carries three distinct artifact kinds, and
a branch's kind should be obvious from its commits: **judgment
candidates** (the change itself, pushed to fire its acceptance matrix),
**resample commits** (empty-delta commits pushed to the same branch for
extra samples — the bot's workflow_dispatch and re-run API calls return
403, so "push again" is the sampling mechanism), and **never-merge
probes** (a deliberate variant diff — one fusion disabled, say — judged
against a non-main base to isolate a mechanism; say "never merges" in
the commit message and never open a PR for one). A `bench/BASE` file,
when present on the branch, names the base ref the workflow checks out
instead of main — that is how a probe measures against its parent
change rather than against main.

## The efficiency pass (v0.7)

A measured audit of every interpreter hot path, integrated in three merged
PRs and gated on this harness. Two headline sources: fast-idiom natives (bit
intrinsics, Bytes readers/bulk appends — the hand-rolled demo versions
became one-line wrappers) and an interpreter sweep (dispatch-loop state
hoisting, write-in-place stack traffic, allocation-free `for` over Int
ranges, scalar structural-hash fast paths, interned single-char strings,
an allocation-free GC mark phase, FMap single-entry buckets without
SipHash, borrow-based string/list natives, `strings.Builder` re-backed by a
`Bytes` buffer, `std.json` over UTF-8 bytes).

Final numbers, complete tree vs the pre-pass binary, interleaved best-of-3
on the reference container (the Method section's "quiet box"; this pass
predates the best-of-4+ floor the Method section above now states, and is
not re-run retroactively — see that section's own note that a claimed win
must beat shared-box noise or be backed by instruction/allocation counts,
which the sheer size of these deltas satisfies regardless):

| target                       | pre-pass | final  | delta      |
|------------------------------|---------:|-------:|-----------:|
| checkers (negamax, 2.04B ops)|  15.74s  | 13.33s | **−15.3%** |
| lisp (interpreter-in-interp) |   2.40s  |  1.93s | **−19.6%** |
| string building              |   0.49s  |  0.22s | **−55%**   |
| map ops                      |   0.22s  |  0.14s | **−36.4%** |
| arith loop (dispatch floor)  |   0.52s  |  0.44s | −15.4%     |
| float loop                   |   0.42s  |  0.34s | −18.9%     |
| enum match                   |   0.29s  |  0.25s | −13.8%     |
| sudoku (bit intrinsics)      |   0.33s  |  0.16s | **−51%**   |
| GC-stress lisp (dev-facing)  |  ~47s    |  ~15s  | **−67%**   |

(Errata: the enum-match and map-ops deltas previously read −14.5% and
−37% — figures that do not follow from the quoted two-decimal times,
presumably computed from unrounded measurements that were not recorded.
The quoted times are the surviving record, so the deltas now match
them: −13.8% and −36.4%.)

A note for release posts: the regression story is worth telling honestly —
the first combined tree measured checkers *+5.6%* even though every
constituent change measured flat-or-better alone. It was a codegen-layout
artifact; a dispatch-core wave (frame-state hoisting et al.) buried it and
turned it into −21% on checkers before the pass shipped.

## The dispatch restructure (H1) and the four-arch gate

The "run() is a codegen lottery" headroom item is resolved for
dispatch-arm changes (the rodata/data-section sub-case remains live on
x86_64-linux — see the Refinement note below). The trigger
was the minification pass's first wave (moving `fft.magnitude` to std):
removing one mid-enum `Native` variant — no semantic change — swung
dispatch-heavy targets ±5–14% on one box and scattered *different*
marked regressions per architecture on the matrix. `Cargo.toml` already
had `lto = true` and `codegen-units = 1`, so the mechanism is
whole-program layout shift: any edit moves every function, and a large
`run()` amplified alignment changes into measured swings.

**H1**: `run()` keeps only compact, frequent arms inline; nine bulky or
rare op bodies moved verbatim behind `#[inline(never)]` (`SetUpvalue`,
`Closure`, `ToString`, `Concat`, `MakeMap`, `MakeRange`,
`MakeStructEmpty`, `ForPrep`, `ForNext`), and `GetGlobal`'s
uninitialized-global error construction into a `#[cold]` factory.
Four-arch verdicts vs main (two interleaved samples per arch; marked
rows only):

| arch           | improvements                        | regressions |
|----------------|-------------------------------------|-------------|
| x86_64-linux   | 8 rows, −3.1..−7.5%                 | none (worst +1.6%) |
| x86_64-windows | 11 rows, −3.7..−7.8%                | none (worst +2.6%) |
| aarch64-macos  | 18 rows, −4..−27%, reproduced ±1%   | none accepted (a lisp +5.1% mark; see the errata note below) |
| aarch64-linux  | none                                | enum_match +4.5% (systematic, below) |

(Errata, macOS lisp +5.1%: the dismissal rests on two samples — the
mark appeared in one and was absent in the other — which was formally
below the ≥3-run majority this file's own macOS measurement protocol
demanded at the time, and is further below the ≥5-run floor the
protocol demands now (raised 2026-07-18). It is retained as an
observation, not an adjudicated verdict; no additional sample was taken
because H1 shipped on the strength of the other three architectures
plus macOS's 18 reproduced improvements.)

The robustness proof: re-applying the same variant removal on top of H1
and judging against H1 itself (a `bench/BASE` probe) is flat on **all
four** architectures — Linux and Windows in single runs, macOS by
multi-sample majority. The dispatch-arm lottery is dead; surface
minification is unblocked.

(Revalidation note, 2026-07-19: Linux and Windows were judged here on a
single run apiece, short of the ≥5-sample floor this file now holds
every local probe to. Per the new-inconclusive-not-negative rule, this
is retained as an observation, not an adjudicated verdict — same
template as the H1 macOS lisp+5.1% errata and the H3 for_range residual
elsewhere in this file. Not re-fired proactively: H1's own per-target
table above already shows the same flat-on-Linux/Windows,
majority-flat-on-macOS shape with more samples per architecture, so a
fresh single-purpose re-run of this specific probe is unlikely to
overturn the qualitative read.)

(Refinement, PR #82: "dead" is scoped to dispatch-arm changes. A
data-section shift alone -- six `&'static str` completion entries plus
one cold branch, no hot code touched -- reproduced +4-6% on
x86_64-linux dispatch-heavy rows (enum_match +5.4/+6.5%, checkers
+5.5/+4.2% across two runner machines) while the other three
architectures stayed flat in both samples. The rodata lottery is alive
on x86_64-linux; the change was merged as an accepted correctness cost,
and this epoch's rebaseline absorbs it. Judge future data-only diffs
accordingly.)

**The per-target binding.** enum_match +4.5% on aarch64-linux
reproduced three times across *two different arm layouts* (H1 and the
rejected hottest-first reorder), so it is a systematic cost of the
compact loop on Neoverse, not a placement roll — that bench executes no
outlined op. Per the PROJECT.md rule, an irreconcilable per-target
disagreement is never accepted as a tradeoff: the op bodies live once
in vm.rs, and an attribute pair binds each target to its
measured-fastest form — `#[inline(never)]` (compact loop) everywhere
except aarch64-linux, where a build.rs-emitted `monolithic_dispatch`
cfg flips them to `#[inline(always)]` and folds the monolith back
together. Non-aarch64-linux binaries are unchanged by the cfg
machinery; aarch64-linux is judged on the matrix like everything else.

(Revalidation note, 2026-07-19: re-verified at the current ≥5-sample
floor via `bench/h1-binding-recheck` — a probe, never merges — whose
build.rs forces `monolithic_dispatch` OFF on aarch64-linux only,
judged against `bench/BASE` = main with the binding on. The branch
carries 8 commits, not 5: the initial push (`79df5da1`) plus 7
empty-commit resamples. Three of those (`183e305`, `1cae75d`,
`c63db13` — "resample 2/5" through "4/5") were pushed within the same
few seconds of each other and their CI runs were cancelled before
producing any data — not discarded for a data-quality reason, they
simply never ran to completion. The other four resamples
(`1507453c` "5/5", `adbe60e5` "6", `726e172e` "7", `d5f25eae` "8/5
(final)" — the renumbering across the gap is this branch's own paper
trail of the cancellations) completed normally, giving 5 valid samples
total across all four tier-1 architectures: 79df5da1, 1507453c,
adbe60e5, 726e172e, d5f25eae, in that order:

| row              | aarch64-linux (head=OFF vs main=ON) | x86_64-linux | x86_64-windows | aarch64-macos |
|------------------|--------------------------------------|--------------|----------------|---------------|
| enum_match       | +5.4/+5.2/+5.3/+5.2/+4.9 (5/5 ⚠)     | +7.0/+3.7/-0.0/+0.3/-0.1 (2/5 ⚠, no consistent direction) | -0.0/+4.0/+0.6/+0.2/-0.9 (1/5 ⚠) | +0.4/+0.2/+0.2/+2.3/+0.2 (0/5 ⚠) |
| closure_churn    | +4.1/+4.0/+4.0/+3.7/+3.7 (5/5 ⚠)     | flat (5/5)   | flat (5/5)     | flat (5/5)    |
| bench_list_churn | +3.4/+3.3/+3.4/+3.1/+3.5 (5/5 ⚠)     | flat (one s1 mark, +5.0) | flat (one s5 mark, -3.2) | flat (5/5) |

Forcing the compact loop back onto aarch64-linux reproduces, tight and
in every one of 5 samples, the cost the binding exists to erase:
enum_match +4.9..+5.4%, closure_churn +3.7..+4.1%, bench_list_churn
+3.1..+3.5% — a systematic Neoverse cost, not a placement roll,
matching (and slightly exceeding) H1's original +4.5% reading. The
other three architectures show no such pattern: at most one or two
isolated single-sample marks apiece, on different rows each time, no
row reproducing adversely across samples — ordinary per-job noise, not
a `monolithic_dispatch` effect, since the probe's build.rs diff is
inert on those targets (the emitted cfg is unchanged there).
x86_64-linux's own scattered enum_match marks (2 of 5 samples,
magnitude +3.7..+7.0%, no consistent direction) are the
already-documented rodata lottery noted earlier in this section, not a
new finding. Confirmed, unchanged: aarch64-linux still needs
`monolithic_dispatch` on, exactly as built. This closes the reopening
recorded under "The floor is uniform across every leg," below —
`bench/h1-binding-recheck` never merges (probe only, no code change)
and needs no further sampling; it is retired per the
branches-live-and-die-within-a-shot policy (2026-07-20, CLAUDE.md
session mechanics).)

**macOS measurement protocol.** macos-14 runners are precise but
per-job biased: an A/A run (identical trees both sides; a
Compare-binaries step proved the two independent builds bit-identical)
measures flat, and large deltas reproduce within ±1% across runs — yet
small (≲6%) layout-dependent deltas flip sign between jobs on the same
binary pair. **Judge macOS marked rows by majority across ≥5 runs,
never fewer — no exception.** (Raised from ≥3, 2026-07-18: the ≥3
floor with a same-decision escape hatch down to two samples is what let
the W2 enum_match dismissal below adjudicate on 1-of-2; the escape
hatch is removed along with the floor increase.) Case law from the
superinstruction wave: a mark that holds direction at consistent
magnitude across all three samples it was actually convicted on is
strong evidence (for_range +4.5/+4.5/+3.9 — beyond anything the
modulation ever sustained; distinguish it from the modulation
signature, the H1-era map_ops +6.2/+5.5 → −8.3 flip) but the floor at
the time of that conviction was ≥3, one short of the current bar — see
the revalidation note under H3, below. This whole characterization is a
property of the macos-14 image bench.yml pins: re-run the A/A
characterization whenever the macOS runner image changes. The
aarch64-macos-15 leg (added 2026-07-18 under the deprecated-is-not-
discontinued rule) starts its own record — its first A/A is the
audit-batch matrix run that introduced it — and inherits the
aarch64-macos label when macos-14 is actually removed (2026-11-02).

**Local single-box probes (the H2/H3-style pre-matrix gate) carry the
same ≥5-sample floor before any keep/drop call**, no exception for an
apparently-clean or apparently-dead result — this closes the second
half of the same gap: the floor above was written for the CI matrix
only, but every local probe this project has run (W1a's local check —
a held wave with no results entry of its own; see HISTORY.md's
"archive/h2-small-list and the W1a hold" — plus H2, H3, and the
post-H1/H3 re-examination probes) used two samples
informally, with no floor stated anywhere. Two samples is now
insufficient for any keep/drop verdict, full stop; a probe that only
gathered two samples before this rule lands has an inconclusive, not
negative, result — see the revalidation note under H2, below.

**The floor is uniform across every leg, not macOS-only** (widened
2026-07-18): x86_64/aarch64-linux and x86_64-windows convictions need
≥5 samples too, even though those legs converge cleanly far more often
than macOS does — a leg being usually-clean is not a reason to demand
less evidence of it when it does show a mark. This reopens H1's
aarch64-linux `enum_match` cost, accepted as real on 3 reproductions
across 2 layouts (H1 sample 1/2, the rejected H1b reorder) — one short
of the current bar — and, more precisely, the `monolithic_dispatch`
per-target binding built to erase it, which was verified clean on
exactly *one* matrix sample. `bench/h1-binding-recheck` (never merges)
re-measures the binding's own effect — forced off vs `bench/BASE` =
main with the binding on — at 5 samples on aarch64-linux; see the
per-target binding note above — confirmed, unchanged.

**The sixth probe: when 5 samples don't resolve, escalate the
*kind* of evidence, not the count.** The floor exists to stop premature
verdicts on too little data, but it is a floor, not a ceiling reached
by counting forever: wall-clock A/B on a shared runner has an
irreducible noise source (scheduler/machine-state jitter), and once
that noise dominates a genuinely marginal signal, a 6th, 7th, or 8th
sample of the *same kind* stops adding resolving power — it just
re-measures the same noise distribution. When 5 samples are in and the
picture still doesn't converge (direction unstable, or a systematic
mechanism can't be independently confirmed), the next probe changes
what's being measured, not how many times: either a deterministic
instrument that removes the noise source entirely (instruction/cache
counts via `perf stat` or cachegrind, immune to scheduler jitter the
way wall-clock timing isn't), or escalation to an entity outside the
automated sampling loop — the user, whose judgment and context the
loop itself can't supply. Two instances, one recognized after the
fact: `bench/h3-probe-no-glc` (isolated the `get_local_const` fusion's
own contribution rather than re-running the aggregate H3-vs-main A/B a
4th and 5th time) was already this pattern before it had a name;
`bench/h1-binding-recheck` (above) is the second, deliberate instance.

**The hypothesis-test ladder — the sixth probe's deterministic-
instrument branch, spelled out.** The bar to leave passive measurement
for an active test: a finding reproducing across two probes. From
there, the loop is hypothesis → test → verdict, not hypothesis → wider
dig: a probe built to confirm or refute one specific, falsifiable
prediction, not a fishing expedition. Two outcomes per test —
*confirmed*: commit to the hypothesis and scope the idiom set *up* to
cover the newly-understood case, per the universality principle above;
*refuted*: form the next hypothesis and test that one specifically, not
a wider unfocused dig. Bounded at four hypothesis-tests total — a fifth
candidate with none confirmed is itself the signal to take the sixth
probe's other branch (escalate to the user) rather than keep guessing.
Keep a scratchpad of each test's data as it accumulates, not only at
the end — not just *whether* to drop or promote a hypothesis early on
partial data, but a slot-by-slot rule for what to spend each probe or
sample on, at either scale (the ≤4-hypothesis ladder, or the ≥5-sample
floor within one hypothesis):

1. **Ground.** The first reading of whatever's currently under test. No
   comparison exists yet.
2. **Differential.** The second reading of the same target gives a
   trend, but two points never confirm or reject anything on their own
   — matches "fewer than 5 is inconclusive" above.
3. **The first real choice, no early exit.** Reprobe/resample the
   current target, or spend the slot on a different one (a different
   hypothesis, a different kind of evidence) *only if* that different
   target would yield more insight right now than another reading of
   the current one. Don't default to resampling from inertia — weigh
   the two options each time.
4. **The same choice, but an early exit is now allowed.** If the
   accreted evidence already compels a decision, decide here rather
   than waiting for the last slot. Otherwise, the slot-3 choice applies
   again — reprobe or switch, whichever yields more insight — and a
   target abandoned at slot 3 is eligible for reconsideration here;
   switching away from something once doesn't permanently disqualify
   it from later slots.
5. **The decisive slot.** Decide if the accreted evidence motivates
   it — commit to the hypothesis (scope the idiom set *up*, uptake the
   change) or change hypothesis entirely, based on what's accumulated.
   Not yet motivated even here: the floor is a floor, not a ceiling —
   the sixth probe above still governs (escalate the kind of evidence,
   or the user), not a mechanical 6th reading.

The same five-step logic governs the hypothesis ladder one slot
shorter, since its own bound is 4, not 5: ground, differential, the
slot-3 choice, then the decisive slot (commit-and-scope-up, or the next
hypothesis) — there is no separate "early exit allowed" slot distinct
from the decisive one at that scale. First instance:
`bench/inline-upvals-x64-probe`, testing whether PR #103's x86_64-linux
`for_range` residual is the representation choice itself vs. an
incidental layout-shift artifact (see the per-target binding note
above for the outcome once read).

**aarch64-macos-15's first A/A** (identical binaries both sides, the
audit-batch-1 run that introduced the leg): macros dead flat (checkers
−0.5%, lisp +0.5%) — consistent with macos-14's own macro behavior.
Small rows showed the same per-job modulation signature macos-14 has,
but *wider* on this first sample: map_ops −8.8%, bench_display +5.5%,
bench_lists −6.4%, method_dispatch +5.0%, bench_join_heavy −3.2%,
bench_env_maps −3.0% — every one an A/A mark (identical source both
sides), so all of it is noise by construction, not a real macos-15
cost. One sample is not enough to characterize the leg's noise
envelope relative to macos-14's — this is the opening data point, not
a conclusion. Accumulate on future matrix runs against this leg.

Two instrument facts worth keeping: release builds are deterministic
(bit-identical across checkouts) only when the checkout paths have
equal length — embedded path lengths shift layout, which is why
`bench.yml` checks out `base/` and `head/` and why "+7.4% between two
builds of identical source" was once measured locally across
different-length paths. And sub-10ms macro targets (reversi, png)
bounce ±4% in both directions on every platform; ignore their marks at
any threshold.

## W2: std.json escape fast path

`std.json`'s string `escape()` gained a clean-string fast path: scan
first, and pass a string through untouched when no character needs
escaping. Judged on the four-arch matrix via bench_json (the row that
exercises parse + stringify); best-of interleaved per ab.py, two
samples where two are listed:

| arch           | sample 1 | sample 2 |
|----------------|---------:|---------:|
| x86_64-linux   |   −4.5%  |   −5.2%  |
| aarch64-linux  |   −7.5%  |    —     |
| x86_64-windows |   −4.9%  |   −6.6%  |
| aarch64-macos  |   −5.8%  |   −3.1%  |

Two adverse marks appeared in single samples — bench_join_heavy +4.4%
on x86_64-windows and enum_match +3.4% on aarch64-macos — and neither
reproduced in sample 2. Multi-sample adjudication (the macOS protocol
above, generalized: judge a mark by majority across samples, never on
one) dismissed both as layout/runner noise. The extra samples were
obtained by pushing empty commits to the bench branch — the bot
account's API calls to workflow_dispatch and re-run return 403, so
"push again" is the sampling mechanism.

(Errata, both adverse marks: each dismissal rested on 1-of-2 samples —
the mark appeared in sample 1 and was absent in sample 2, for both
x86_64-windows bench_join_heavy +4.4% and aarch64-macos enum_match
+3.4% — which was already below the ≥3-run majority the protocol
demanded at the time, and is further below the ≥5-run floor the
protocol demands now. Same template as the H1 lisp+5.1% errata above:
both are retained as observations, not adjudicated verdicts. No
re-fire is queued — W2's merge rested on bench_json's win reproducing
in 7 of 7 read job-samples across all four arches, not on either row,
so the mis-adjudication did not change the outcome; it is recorded
here because aarch64-macos's mark is the concrete instance that
motivated tightening the floor, not because either verdict is in
doubt.)

## Epoch: the bench re-specification (consistency pass)

Every bench file now opens with a measurand header (`// Bench: ...`)
stating exactly what the row measures, and counted `while i < N`
scaffolding loops were converted to the modern `for i in 0..N` range
idiom. Only loops whose bookkeeping IS the measurand keep the `while`
shape, and say so in their header: arith_loop (the deliberate
while-loop dispatch row) and float_loop's escape loop (its index is the
payload and escapes the loop). Hand-rolled idioms that ARE the workload
also stay and say so (bitwise_masks' one-bit popcount — the
count_ones intrinsic would delete the thing being timed). Two rows were
re-specified outright: bench_join_heavy previously duplicated
string_build (a builder fill plus repeated `build()`; it performed no
joins) and is now a real join-path bench
(`strings.Builder.push_joined` + `List.join` row assembly), and
`bench/for_range.soc` is new — the fused ForNextRange range-literal
loop, the modern counted-loop dispatch floor, arith_loop's counterpart.

The conversions are stdout-identical against the pre-conversion files
under the same binary — no checksum moved. Wall times DID move: a
for-range loop dispatches different ops than a while loop, so every
converted row's absolute time (and its share of loop overhead) is
re-specified. **The bridge errata is the conversion commit's own
four-arch matrix run**: the interpreter source is identical on both
sides (the Compare-binaries step reports the binaries bit-identical),
so that run's delta table prices exactly the workload
re-specification, per row per architecture. Numbers recorded before
the conversion are comparable to numbers after it only through this
table:

| converted row | x86_64-linux | aarch64-linux | x86_64-windows | aarch64-macos |
|---------------|-------------:|--------------:|---------------:|--------------:|
| bench_call_return | -13.9% | -9.5% | -12.3% | -9.5% |
| bench_deque | -8.1% | -7.0% | -7.5% | -9.1% |
| bench_display | -0.0% | +0.1% | -0.8% | -0.3% |
| bench_env_maps | -3.8% | -3.3% | -2.7% | -5.1% |
| bench_json | -1.9% | +0.4% | -0.1% | +0.2% |
| bench_list_churn | -14.1% | -9.8% | -7.8% | -13.5% |
| bench_lists | -3.2% | -2.2% | -3.4% | -2.2% |
| bitwise_masks | -5.8% | -4.4% | -3.0% | -4.5% |
| closure_churn | -18.0% | -13.1% | -12.9% | -16.8% |
| enum_match | -15.4% | -11.4% | -11.5% | -13.8% |
| float_loop | -1.9% | -0.6% | +0.0% | -0.7% |
| list_ops | -23.3% | -14.1% | -21.7% | -17.8% |
| map_ops | -6.4% | -7.9% | -7.2% | -6.9% |
| method_dispatch | -26.3% | -17.6% | -23.2% | -20.0% |
| string_build | -18.8% | -11.6% | -16.1% | -13.8% |
| string_interp | -12.6% | -9.0% | -9.8% | -10.2% |

New-baseline rows (head seconds; no valid cross-epoch delta exists):
bench_join_heavy 0.1648 / 0.1924 / 0.2112 / 0.1392 (the old row measured
a different workload) and for_range 0.1536 / 0.1579 / 0.1602 / 0.1229
(added in this epoch), in the same arch order as the table.

Controls behaved as the bridge premise demands (run 29625034983,
2026-07-18): arith_loop, the kept-while row, sat at -0.3/-0.6/+0.6/+0.5
across the four arches; the unchanged demo macros were flat everywhere
except one x86_64-linux checkers -8.0% -- provably runner noise, since
the job's Compare-binaries step printed `binaries: BIT-IDENTICAL` and
the demo sources (and enforced checksums) were identical on both sides.
The converted rows' uniform improvements are the fused range loop's
cheaper bookkeeping, not an interpreter change -- that is precisely the
workload re-specification this table exists to bridge.

## W3: the List.sum() native

`List.sum()` became a native (one pass over the backing storage,
checked_add, `List[Int]`-constrained at check time); `lists.sum` is its
one-line wrapper per the popcount precedent. bench_lists on the
four-arch matrix (run 29628798164): x86_64-linux −58.8%, aarch64-linux
−56.6%, x86_64-windows −55.2%, aarch64-macos −58.8% (local samples
−56.9/−57.5 — CI reproduced them everywhere). No adverse mark on any
arch. x86_64-linux's favorable-only spread on dispatch rows that run
(enum_match −7.3% etc.) was the rodata lottery rolling the good
direction and is not credited to the change.

(Revalidation note, 2026-07-19: this verdict rests on 1 CI sample per
arch plus 2 local samples, short of the ≥5-sample floor this file holds
every other verdict to. Per the new-inconclusive-not-negative rule this
is retained as an observation, not formally re-adjudicated — the
magnitude here (−55..−59%, reproduced identically across every sample
taken) is far enough above shared-box noise that a fresh 5-sample run
is unlikely to overturn the qualitative read, matching this file's own
precedent for not re-firing an already-tight directional result. Not
re-fired proactively: no code has changed in `List.sum()` since.)

## H3: superinstructions — and the macOS for_range residual

Four fused ops chosen from a dynamic pair profile over ~2.5B dispatches
(`get_local_const`, `get_local2`, `get_global_const`,
`get_local_test_variant`; fusion after jump patching, never across a
jump target; the measured-rejected compare-and-branch shape stays
excluded — no fused op contains control flow). Gate-row deltas vs main
on the four-arch matrix (samples 1/2, run 29632669449 + empty-commit
resamples):

| arch           | float_loop | enum_match | method_dispatch | arith_loop | bitwise |
|----------------|-----------:|-----------:|----------------:|-----------:|--------:|
| x86_64-linux   | −4.0       | −4.8       | −2.9            | −0.4       | −3.8    |
| aarch64-linux  | −7.2       | −4.5       | −4.0            | −9.1       | −10.2   |
| x86_64-windows | −10.9/−11.8| −8.6/−9.1  | −6.2/−7.8       | −10.8/−9.7 | −9.7/−11.5 |
| aarch64-macos  | −5.6/−4.8  | −7.9/−10.2 | −3.6/−3.7       | −5.8/−7.9  | −6.0/−5.4 |

aarch64-linux runs the fused arms under the `monolithic_dispatch`
binding and posted the broadest sweep of the four. Windows sample-1
bench_display +4.4% and macOS string_interp +3.7% both failed to
reproduce (noise per the multi-sample protocol).

**The residual: aarch64-macos for_range +4.5/+4.5/+3.9 — adverse three
samples running, direction 3/3.** Real by the same standard that judged
H1's aarch64-linux enum_match cost real. The per-target-binding remedy
was then pursued to the end of the evidence and does not exist here:

- A probe branch (`bench/h3-probe-no-glc`, base = full H3 via
  `bench/BASE`) removed only the `get_local_const` fusion — the one
  fused op in for_range's profile. Two samples: for_range recovered
  only −2.3/−2.1% (sub-floor twice — the fusion is at most half the
  cost), while the removal *reproducibly regressed* the macOS rows that
  fusion carries: bench_call_return +6.4/+6.7%, bitwise_masks
  +5.3/+5.1%, bench_lists +3.1/+3.0%, checkers +3.0/+3.2%. The fusion
  the signal pointed at is load-bearing; gating it off trades one +4%
  row for four.
- Disabling fusion entirely on macOS is worse by arithmetic: it
  forfeits the −4..−10% sweep on a dozen rows to flatten one.
- Chasing the unexplained ~2% into arm placement is the dice-chasing
  the H1b negative result prohibits.

Conclusion, per the universality principle's own remedy clause: each
target is bound to its measured-fastest form, and for aarch64-macos the
measured-fastest form **is** full H3 — both finer-grained alternatives
measured worse. The for_range +4% is recorded here as the residual of
macOS's own fastest configuration, with the probe evidence above as the
receipt that the binding remedy was tried, not skipped. Do not re-open
without new evidence (a new fusion set, a toolchain change, or an
M-series microarchitectural insight would qualify).

(Revalidation note, 2026-07-18: this residual was convicted on 3
samples (direction 3/3, magnitude 3.9–4.5%) under the ≥3-run floor in
force at the time; the floor is now ≥5. Two backfill samples were
planned on the then-still-live `bench/h3-superinstructions` branch to
bring this to 5 without re-opening the merged decision.

Update, 2026-07-19: that backfill never happened. `bench/
h3-superinstructions` was merged as PR #89 and deleted along with it —
it no longer exists — and its commit history at merge time
(`64dbdc3`/`dca2128`/`6c5e993`/`4bb1a5b` — four commits total,
implementation and documentation together, not one per sample) carries
no resample commits beyond the original 3 samples this residual was
convicted on. The
promise went unfulfilled, not merely undocumented; this residual's own
data is still exactly the 3 samples above, short of the current ≥5
bar. Per the new-inconclusive-not-negative rule, it is retained as an
observation, not an adjudicated verdict — same template as the H1
macOS lisp+5.1% errata above. This does not reopen the Conclusion
above, though: that decision doesn't rest on this residual's precise
sample count, it rests on the no-GLC probe's independent finding that
disabling the one implicated fusion recovers less than half the
residual while reproducibly regressing four other rows — evidence that
holds regardless of how many samples convicted the residual itself.
Not re-fired proactively, matching this file's own precedent for the
analogous small-list DROP gap below: no code has changed in the
relevant mechanism since, and a fresh 5-sample run is unlikely to
overturn an already-tight 3-for-3 directional read. The no-GLC probe
(two samples, cited above) remains a mechanism diagnostic that fed this
decision, not a live shippable claim in its own right; it stays
grandfathered rather than re-fired for the same reason.)

## Inline upvalues: a reopened negative result, KEPT with a two-target binding

`Obj::Closure`'s upvalue storage (previously a bare `Vec<Handle>`,
heap-allocated on every closure construction) becomes `UpvalStorage`: an
inline-slots-or-spill enum, `InlineUpvals::Inline { len, slots:
[Handle; 2] }` for closures capturing ≤2 upvalues (`bench/closure_churn`'s
own shape and ordinary practice) or `InlineUpvals::Many(Vec<Handle>)`
beyond that — same 24 bytes as the `Vec` it replaces, so `Obj` stays
exactly 64 bytes. This reopens and reverses the "Inline ≤2 upvals wins
its micro but loses the dispatch-loop codegen lottery" entry that used
to stand in Negative results below, on the reflexive-codification
audit's flag that H1 killed that lottery premise on Linux/Windows.

**First four-arch matrix (`bench/inline-upvals`, base cf4f8630 onward, 5
samples vs main) found two real per-target costs, not one.**
`closure_churn` won big everywhere (−10% to −19%, tight and
reproducible on every sample) except aarch64-linux, which instead
showed a broad, tight regression across enum_match/for_range/
bench_call_return/png (+3.2–5.2%, 4/4 marked every sample) — the same
inlined-op-body-complexity sensitivity `monolithic_dispatch` already
routes around on that target (`GetUpvalue`/`SetUpvalue`/`Closure` all
inline into the Neoverse monolith there). Fixed the same way: aarch64-
linux keeps plain `Vec<Handle>`, reusing `monolithic_dispatch` as the
binding predicate (commit cf4f8630) since the mechanism is the same one
that cfg already exists for.

**That fix left a second residual: x86_64-linux's `for_range` marked
+2.8/+6.4/+6.2/+6.3/+9.1% across all 5 samples (4/5 over the 3% floor,
the 5th same-direction but sub-floor) — despite `for_range` touching no
closures or upvalues at all.** Per Roxy's direction for exactly this
situation ("if you keep replicating the same finding, maybe that's the
case where you should actually test it"), this was tested rather than
assumed to be the project's known whole-program layout-shift ("rodata
lottery") artifact class:

- **Hypothesis:** the mark is a real cost of `InlineUpvals`'s
  representation on x86_64-linux too (a different underlying reason
  than aarch64-linux's, but possibly the same `Vec<Handle>` remedy) —
  not an incidental layout artifact.
- **Test:** `bench/inline-upvals-x64-probe` (never merges), branched
  from `bench/inline-upvals`'s tip, forced `Vec<Handle>` on
  x86_64-linux via a second, deliberately distinct predicate (an inline
  `#[cfg(any(monolithic_dispatch, all(target_arch = "x86_64",
  target_os = "linux")))]` for the probe), `bench/BASE` pinned to
  `bench/inline-upvals`'s own tip for single-variable isolation.
  Gathered the full ≥5-sample floor via the hypothesis-test ladder's
  slot protocol (ground, differential, then reprobe-vs-switch each
  slot after — see PROJECT.md and the sixth-probe-doctrine paragraph
  above): x86_64-linux `for_range` **−5.8% / −5.8% / −1.0% / −5.7% /
  −6.0%**, direction 5/5 favorable (reverting to `Vec<Handle>` reverses
  the regression every time), marked 4/5 — the mirror image of the
  original discovery's own noise profile (magnitude-for-magnitude,
  down to which single sample landed sub-floor). **CONFIRMED**: the
  representation choice itself is the cause on x86_64-linux too, not a
  layout-shift artifact.

**Formalized as a second, distinctly-named build.rs cfg,
`upvals_vec_handle`** — deliberately *not* folded into
`monolithic_dispatch`, even though both targets land on the same
`Vec<Handle>` form: `monolithic_dispatch` is specifically vm.rs's own
dispatch-loop-arm-inlining binding (why aarch64-linux's compact loop
flips to a monolith), a mechanism x86_64-linux does not share and
should not silently inherit by reusing the cfg name (it would also flip
vm.rs's own dispatch arms on x86_64-linux, an unrelated and untested
change). aarch64-linux now rides both cfgs, each for its own reason;
x86_64-linux rides only the new one; x86_64-windows and aarch64-macos
keep `InlineUpvals`.

**Fresh four-arch matrix on the formalized binding, 5 samples vs main
(run 29671924853 onward), confirms flat-or-better on every row, every
architecture, with one flagged single-sample exception addressed
immediately below:**

`for_range` (the row that mattered):

| arch | s1 | s2 | s3 | s4 | s5 |
|------|---:|---:|---:|---:|---:|
| x86_64-linux | −0.3% | −0.1% | +0.4% | −0.1% | +3.0% |
| aarch64-linux | +0.4% | +0.0% | +0.1% | +0.0% | +0.1% |
| x86_64-windows | +0.9% | +2.3% | +1.0% | +5.0% ⚠ | −0.7% |
| aarch64-macos | −0.1% | −0.3% | −0.1% | −2.6% | −0.6% |

x86_64-linux's original residual is gone (mixed sign, the one
borderline +3.0% reading not even marked by ab.py's own threshold).
aarch64-linux stays dead flat every sample (unaffected, as expected —
its binding never changed). aarch64-macos stays flat and
same-direction throughout. x86_64-windows marked once (sample 4,
+5.0%) with no consistent direction or magnitude across the other four
readings (+0.9/+2.3/+1.0/−0.7) bracketing it on both sides — read as an
isolated per-job excursion, not a new residual, by the same
multi-sample standard this file already applies to macOS.

`closure_churn` (the real win), both non-Linux targets, all 5 samples:

| arch | s1 | s2 | s3 | s4 | s5 |
|------|---:|---:|---:|---:|---:|
| x86_64-windows | −16.2% | −16.2% | −16.5% | −18.7% | −17.1% |
| aarch64-macos | −11.9% | −10.9% | −10.2% | −10.2% | −10.3% |

Tight and reproducible on both targets, every sample. Every other
single-sample mark across the 20 job-samples in this final matrix
(bitwise_masks windows-s3, bench_deque linux-s4, string_build
windows-s4, a cluster of macos-s5 macro rows) scattered across
different rows each time with no row repeating in the same direction
twice — the ordinary per-job noise signature this file already
documents for shared runners, sub-10ms macros, and macos-14 (see the
macOS measurement protocol above), not a systematic cost of the
binding.

**Verdict: KEEP.** Both the mechanism (inline-slots-or-spill upvalue
storage) and the per-target binding (two Linux targets on
`Vec<Handle>` via `upvals_vec_handle`, each for its own measured
reason; x86_64-windows and aarch64-macos on `InlineUpvals`) are
confirmed at the current ≥5-sample floor, on both the probe that
isolated the hypothesis and the fresh matrix that verified the
formalized binding. First instance of the hypothesis-test ladder
(PROJECT.md, under the ≥5-sample-floor bullet) reaching a CONFIRMED
verdict end to end — the slot-by-slot record is the per-sample
breakdown above; each resample commit on
`bench/inline-upvals-x64-probe` was an empty-commit re-fire carrying no
further numeric detail of its own, so the record above is already the
full one, not a pointer to more. `bench/inline-upvals-x64-probe` is
retired per the branches-live-and-die-within-a-shot policy (2026-07-20,
CLAUDE.md session mechanics).

## Negative results (measured, rejected — do not re-attempt without new evidence)

- GC `next_gc` pacing `(live*2).max(4096)` is already the local optimum in
  both directions.
- Boxing the FMap index loses to the extra pointer-chase on the map hot path.
- Niche-packing `Obj` (dropping `#[repr(u8)]`) regresses match-heavy targets.

  (These three entries predate this file's sampling-discipline
  conventions — no percentage, sample count, or date was recorded for
  any of them, unlike every entry below. Unfalsifiable as written; a
  fresh measurement under the current ≥5-sample floor would either
  reconfirm them with real numbers or reopen them, but none is queued
  proactively — re-attempt only if a reason to revisit one actually
  arises.)
- A fused compare-and-branch peephole: sound, but the same codegen lottery
  swamps the saved dispatch. **Re-examined post-H1/H3 (2026-07-18), per
  the reflexive-codification audit that flagged this entry's premise —
  the dispatch codegen lottery — as killed on Linux/Windows by H1.**
  New evidence, not a re-litigation of the old: a fresh implementation
  (`EqJumpIfFalse`/`LtJumpIfFalse`/`LeJumpIfFalse`/`GtJumpIfFalse`/
  `GeJumpIfFalse`, five fused ops, same jump-safety machinery H3 uses —
  each op computes its comparison and branches directly, popping both
  operands without ever materializing the Bool on the stack, unlike the
  unfused pair's separate compare-then-`JumpIfFalse`; the one compiler
  subtlety worth keeping on record if this is ever rebuilt: the
  fuse-time jump-offset remap needs a 2-old-slot base distance for
  these five ops, not the 1-slot base every other fused/jump op uses,
  because the offset was captured from a `JumpIfFalse` that sat one old
  slot *after* the pair's own recorded index — getting that wrong
  silently miscompiles jump targets rather than failing loudly)
  was built and measured on the current x86_64-linux post-H1/H3 tree, 5
  local samples (the current floor): arith_loop +11.4..+13.2%,
  float_loop +5.9..+6.8%, list_ops +3.9..+11.3%, bitwise_masks
  +2.3..+5.5% (4/5 marked), string_build +2.8..+7.0% (4/5 marked) — a
  clean, reproducible **regression**, not noise. The decisive fact:
  `bitwise_masks` and `list_ops` have *zero* compare→branch pairs in
  their bytecode (confirmed by the same dynamic pair-profiling method
  H3 used) yet regress exactly as reproducibly as `arith_loop`, which
  is 100% fusable — proving the cost is the five new `run()` match arms
  shifting whole-function codegen layout, not a per-fusion-site cost.
  This is H1's original mechanism in mirror image: H1 was triggered by
  *removing* an arm; this shows *adding* arms revives an equivalent
  layout sensitivity, post-H1. DROP reconfirmed, on new grounds, not
  the old lottery premise — the fusion itself was never the problem in
  either era; arm-count churn to `run()` is. Full implementation detail
  is above, not on a branch — `probe-cmp-branch` (never merges) is
  retired per the branches-live-and-die-within-a-shot policy (2026-07-20,
  CLAUDE.md session mechanics): a probe's job ends when its finding is
  written up, not when someone later judges the ref safe to delete.
  **Scope note: measured on x86_64-linux only** — the other
  three architectures were not run, so this DROP rests on one arch's
  data, unlike this file's other four-arch verdicts. H1 itself
  establishes that this class of layout regression scatters
  *differently* per architecture, so x86_64-linux's result doesn't
  predict the others', and the per-target-binding remedy this file
  reaches for elsewhere (`monolithic_dispatch`, `upvals_vec_handle`)
  was neither tried nor ruled out here. **Standing watch:** append a
  dated sighting here whenever `run()`'s arm count changes again (a new
  fusion, a new op) or another architecture's matrix run shows a
  zero-fusable-pair control row regressing the same way arith_loop did
  here — the signature that distinguished this finding from the old
  dispatch-lottery premise. Two such sightings justify re-firing the
  four-arch matrix; none have landed, so it stays unfired.
  Sightings so far: none.
- Hottest-first arm reordering inside the compact `run()` (H1b): did not
  fix aarch64-linux's systematic enum_match cost (still +4.6%, plus a
  new map_ops +4.4% there) and broke x86_64-linux (enum_match −3.1% →
  +4.6%, bench_display +6.7%). Arm order on top of H1 is a pure dice
  roll; H1's source order stands.
- An inline-small-list `Obj::List` representation (`ListInline { len: u8,
  slots: [Value; 3] }` as a second flattened variant — N=3 the largest
  capacity keeping `Obj` at its existing 64 bytes, chosen because 97.3%
  of lists die at len ≤ 3 in the movegen-heavy instrumentation run.
  Every list constructor goes through one `list_from_vec` entry point
  that inlines by the source `Vec`'s *capacity*, not its length — a
  native that pre-sizes before filling (map, filter, zip, entries — the
  rooting discipline needs the list heap-resident while a user callback
  runs) keeps its heap `Vec` rather than having a deliberately-sized
  allocation thrown away and re-grown push by push. Every read (index,
  iterate, equal, hash, display) goes through one `list_slice` view so
  the two representations stay observably identical; a spilled list
  never converts back to inline on shrink, by design — no thrash at the
  boundary.) The mechanism works: bench_list_churn improved
  −7.3%/−7.8% across two interleaved samples. The *target* did not:
  checkers moved −0.2%/−0.1% — its 13.0M movegen allocations are real
  (measured: 96.9% of lists die at len ≤ 2) but tcache-cheap, so
  allocation *count* was never the cost — and bench_display regressed
  reproducibly (+8.2%/+4.5%, container display paying the
  representation branch). Dropped per the pre-registered gate.
  **Standing watch (user-directed, 2026-07-18) — this entry is softer
  than its neighbors: track the signal, don't just refrain.** The
  mechanism measured real (that reproducible list_churn −7%); only the
  workload was wrong. Whenever a new bench, demo, port, or
  implementation type shows churn-bound list behavior — allocation
  cost that a small-list representation would erase, the shape
  bench_list_churn rewards — append the sighting here, dated, with
  the row that showed it. Three sightings across *different* cases
  (matching probe-cmp-branch's own numeric threshold above, scaled up
  since H2's original DROP rested on more evidence than that probe's
  did) push H2 back over the edge for a fresh look;
  the full implementation detail needed to rebuild it is above, not on
  a branch — `archive/h2-small-list` is retired per the
  branches-live-and-die-within-a-shot policy (2026-07-20, CLAUDE.md
  session mechanics), same as `probe-cmp-branch` above: a reopening
  starts from this entry's own detail, not from checking out a ref that
  was already 80+ commits stale before it went.
  Sightings so far: none beyond bench_list_churn itself.
  (Revalidation note, 2026-07-18: the local A/B behind this DROP used
  two samples — checkers −0.2%/−0.1%, list_churn −7.3%/−7.8%, display
  +8.2%/+4.5% — under the pre-floor informal practice; the local-probe
  floor is now ≥5. Per the new-inconclusive-not-negative rule this
  entry's data is short of the bar, though the direction (checkers
  flat, list_churn/display both moving the same way twice) is
  consistent enough that a fresh 5-sample local A/B is unlikely to
  overturn the qualitative read. Not re-fired proactively — no code
  changed since, and the watch mechanism already exists to catch a
  reason to look again; a fresh probe is warranted if and when a
  sighting is actually logged here, not preemptively.)

## Known headroom (identified, not yet taken)

Both items the v0.8 pass identified are now consumed: the
movegen-allocation item was measured and rejected (the inline-small-list
entry above — the pool is real but allocator-cheap), and the
superinstruction item was executed as the four fused operand-fetch ops
(a dynamic *pair* profile replaced the original static "45% hot triple"
estimate; judged, like every dispatch change, on the four-arch matrix).
Nothing further is currently identified — new items come from fresh
profiling, not from this list's history.
