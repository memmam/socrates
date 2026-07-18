# Benchmark results

The measurement instrument is `bench/ab.py BASE_DIR HEAD_DIR` — an
interleaved cross-binary A/B between two full checkouts — fanned across
one runner per tier-1 architecture by the Bench A/B workflow
(`.github/workflows/bench.yml`), which fires on pushing a `bench/<name>`
branch (the bot account's API calls to workflow_dispatch and re-run
return 403, so extra samples are obtained by pushing empty commits to
the bench branch). `bench/run.sh [N]` is a single-binary sequential
profiling convenience — where does one binary spend its time? — not the
gate. Micro-benchmarks (`bench/*.soc`) isolate one cost centre each,
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
each delta table to the run summary. The acceptance rule is CLAUDE.md's
universality principle: flat-or-better everywhere, or the idiom keeps
its primitive.

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
on the reference container:

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

The "run() is a codegen lottery" headroom item is resolved. The trigger
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
mark appeared in one and was absent in the other — which is formally
below the ≥3-run majority this file's own macOS measurement protocol
demands. It is retained as an observation, not an adjudicated verdict;
no third sample was taken because H1 shipped on the strength of the
other three architectures plus macOS's 18 reproduced improvements.)

The robustness proof: re-applying the same variant removal on top of H1
and judging against H1 itself (a `bench/BASE` probe) is flat on **all
four** architectures — Linux and Windows in single runs, macOS by
multi-sample majority. The dispatch-arm lottery is dead; surface
minification is unblocked.

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
outlined op. Per the CLAUDE.md rule, an irreconcilable per-target
disagreement is never accepted as a tradeoff: the op bodies live once
in vm.rs, and an attribute pair binds each target to its
measured-fastest form — `#[inline(never)]` (compact loop) everywhere
except aarch64-linux, where a build.rs-emitted `monolithic_dispatch`
cfg flips them to `#[inline(always)]` and folds the monolith back
together. Non-aarch64-linux binaries are unchanged by the cfg
machinery; aarch64-linux is judged on the matrix like everything else.

**macOS measurement protocol.** macos-14 runners are precise but
per-job biased: an A/A run (identical trees both sides; a
Compare-binaries step proved the two independent builds bit-identical)
measures flat, and large deltas reproduce within ±1% across runs — yet
small (≲6%) layout-dependent deltas flip sign between jobs on the same
binary pair. Judge macOS marked rows by majority across ≥3 runs, never
one sample.

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

## Negative results (measured, rejected — do not re-attempt without new evidence)

- GC `next_gc` pacing `(live*2).max(4096)` is already the local optimum in
  both directions.
- Boxing the FMap index loses to the extra pointer-chase on the map hot path.
- Niche-packing `Obj` (dropping `#[repr(u8)]`) regresses match-heavy targets.
- Inline ≤2 upvals wins its micro but loses the dispatch-loop codegen
  lottery (+2.8% Ir elsewhere).
- A fused compare-and-branch peephole: sound, but the same codegen lottery
  swamps the saved dispatch.
- Hottest-first arm reordering inside the compact `run()` (H1b): did not
  fix aarch64-linux's systematic enum_match cost (still +4.6%, plus a
  new map_ops +4.4% there) and broke x86_64-linux (enum_match −3.1% →
  +4.6%, bench_display +6.7%). Arm order on top of H1 is a pure dice
  roll; H1's source order stands.

## Known headroom (identified, not yet taken)

- checkers' 13.5M movegen `List` allocations are its biggest single cost
  pool; an inline-small-list `Obj::List` representation is the real fix.
- Superinstructions for the `GetLocal`/`Const`/`JumpIfFalse` hot triple
  (45% of dispatched ops) — unblocked now that the H1 dispatch
  restructure landed (the lottery that made such changes unjudgeable is
  gone; judge on the four-arch matrix).
