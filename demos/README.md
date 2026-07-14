# The Fable demos

Ten programs, each a self-contained showcase of the language doing real
work. Every demo is deterministic, pins its complete output with golden
`//?` directives, and passes under GC stress:

```sh
cargo build --release
./target/release/fable demos/lisp/main.fable      # run one
./target/release/fable test demos                 # golden-test all ten
FABLE_GC_STRESS=1 ./target/release/fable test demos
```

| Demo | What it does | Worth seeing |
|------|--------------|--------------|
| [`lisp/`](lisp/) | A mini-Lisp: reader, evaluator, and five sample programs (factorial, fib, a Lisp-level `map`, closures, a 100k-iteration loop). | Fable's TCO reaches *through* the interpreter — the tail-recursive Lisp loop runs in constant stack. `try()` turns VM panics into Lisp error values. |
| [`spreadsheet/`](spreadsheet/) | Formulas with a Pratt parser, dependency-driven evaluation, memoization, and spreadsheet-faithful error values. | Busy-set cycle detection (`#CYCLE!` propagates, then *heals* when the cycle is edited away). Stress-tested on 3,000-cell chains. |
| [`regex/`](regex/) | A backtracking regex engine: literals, classes, anchors, `* + ? \|`, groups, escapes. 56 match tests + grep mode with underlined matches. | The matcher is continuation-passing style — `\|\|` short-circuiting *is* the backtrack stack, in ~65 lines. |
| [`dungeon/`](dungeon/) | A seeded roguelike dungeon generator: rooms, L-corridors, BFS shortest path drawn onto the ASCII map. | Found a real interpreter bug: v0.5's `math.seed` made seeds 42 and 43 generate identical dungeons. Fixed in v0.6; see `NOTES.md`. |
| [`mdsite/`](mdsite/) | A static site generator: markdown → templated HTML site, three sample pages, build report. | A block-level state machine plus an inline-span scanner; the committed `out/` pages are byte-identical to a fresh build. |
| [`csvql/`](csvql/) | A query language over CSV: `select city, pop where continent == Asia group by continent order by avg pop desc limit 3`. | Group-by buckets keyed by enum values in a Map (structural hashing + insertion order = deterministic reports). Every malformed query degrades to one tidy error line. |
| [`checkers/`](checkers/) | Full English draughts (forced captures, multi-jumps, kings) with a negamax alpha-beta engine. | A complete 106-ply self-play game — every move, eval, and node count pinned as ~200 golden lines. Ends in an honest threefold-repetition draw. |
| [`plot/`](plot/) | A function plotter: SVG line charts with 1/2/5 nice ticks and collision-dodged labels, a 75-stroke spirograph, terminal sparklines. | Regenerates both committed SVGs byte-for-byte. The palette follows dataviz accessibility guidance. |
| [`sudoku/`](sudoku/) | Naked-singles propagation + most-constrained-cell backtracking over three classic puzzles (including Inkala's "hardest"). | The verifier is independent of the solver — the spec deliberately corrupts a solved board to prove it. Trail-based undo restores boards byte-for-byte. |
| [`wfc/`](wfc/) | Wave-function collapse: learns tile adjacency from ASCII samples, generates new textures by entropy-driven constraint propagation. | The spec pins the *contract*, not just output: zero adjacency violations, same-seed determinism, and a provably impossible tile set that must exhaust its seed budget. |

## Where they came from

The demos were written against **v0.5** by ten independent authors with a
double brief: make something interesting, and surface every papercut. Each
was then verified by a separate reviewer following only its README. Their
issue reports — deduplicated, triaged, and answered — are in
[`NOTES.md`](NOTES.md); the fixes they drove became v0.6, and the demos
were then modernized to use what they'd asked for. The narrative version
is [book chapter 10](../book/10-field-test.md).
