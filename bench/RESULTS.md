# Benchmark results

The harness (`bench/run.sh [N]`) times each target N times and reports the
minimum wall-clock (least-noise estimator). Micro-benchmarks isolate one
cost centre each; macros are the heavy demo mains and the spec suite. Every
program prints a checksum line, so a wrong-answer "optimization" cannot pass
as a win.

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
(`.github/workflows/bench.yml`, run by hand with the branch under test)
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
| map ops                      |   0.22s  |  0.14s | **−37%**   |
| arith loop (dispatch floor)  |   0.52s  |  0.44s | −15.4%     |
| float loop                   |   0.42s  |  0.34s | −18.9%     |
| enum match                   |   0.29s  |  0.25s | −14.5%     |
| sudoku (bit intrinsics)      |   0.33s  |  0.16s | **−51%**   |
| GC-stress lisp (dev-facing)  |  ~47s    |  ~15s  | **−67%**   |

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
| aarch64-macos  | 18 rows, −4..−27%, reproduced ±1%   | none real (a lisp +5.1% mark did not reproduce) |
| aarch64-linux  | none                                | enum_match +4.5% (systematic, below) |

The robustness proof: re-applying the same variant removal on top of H1
and judging against H1 itself (a `bench/BASE` probe) is flat on **all
four** architectures — Linux and Windows in single runs, macOS by
multi-sample majority. The lottery is dead; surface minification is
unblocked.

**The accepted tradeoff.** enum_match +4.5% on aarch64-linux reproduced
three times across *two different arm layouts* (H1 and the rejected
hottest-first reorder), so it is a systematic cost of the compact loop
on Neoverse, not a placement roll — that bench executes no outlined op.
Accepted deliberately: one microbench on one architecture, against
broad reproduced wins on the other three and a lottery-free base for
every future change.

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
