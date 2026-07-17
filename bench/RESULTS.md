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

## Negative results (measured, rejected — do not re-attempt without new evidence)

- GC `next_gc` pacing `(live*2).max(4096)` is already the local optimum in
  both directions.
- Boxing the FMap index loses to the extra pointer-chase on the map hot path.
- Niche-packing `Obj` (dropping `#[repr(u8)]`) regresses match-heavy targets.
- Inline ≤2 upvals wins its micro but loses the dispatch-loop codegen
  lottery (+2.8% Ir elsewhere).
- A fused compare-and-branch peephole: sound, but the same codegen lottery
  swamps the saved dispatch.

## Known headroom (identified, not yet taken)

- `run()` is a codegen lottery (±3–9% Ir from any arm-set change); a
  computed-goto / tail-call dispatch structure would de-risk future work.
- checkers' 13.5M movegen `List` allocations are its biggest single cost
  pool; an inline-small-list `Obj::List` representation is the real fix.
- Superinstructions for the `GetLocal`/`Const`/`JumpIfFalse` hot triple
  (45% of dispatched ops) — needs the dispatch restructure first.
