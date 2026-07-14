# sudoku — constraint propagation + backtracking search, in Fable

A complete sudoku solver. Naked-singles propagation fills every cell that
has exactly one legal digit, looping to a fixpoint; when logic alone
stalls, a depth-first search guesses at the **most-constrained cell**
(fewest candidates first) and backtracks on contradiction. The board is
one shared mutable grid — placements are logged on a trail so a failed
branch unwinds exactly what it added.

Three inline puzzles show the spread: an easy one that propagation solves
outright (zero guesses), Arto Inkala's 2010 "world's hardest sudoku", and
a 17-clue puzzle from Gordon Royle's minimal-sudoku collection (17 clues
is the proven minimum for a unique solution). Every answer is re-checked
by an independent verifier — all 27 units and every original given — and
the run prints search statistics per puzzle.

About 400 lines of Fable in four files:

| File | What it does |
|------|--------------|
| `board.fable` | the `Board` struct (81 cells + per-row/column/box "used" tables so candidate checks are O(1)), parsing with validation, pretty-printing, and the independent `verify` |
| `solver.fable` | naked-singles propagation, the MRV cell chooser, trail-based backtracking search, and `Stats` (assignments / guesses / backtracks) |
| `main.fable` | the three puzzles, side-by-side puzzle/solution rendering, `?`-based error plumbing, an optional `--time` flag |
| `spec.fable` | golden tests: parser rejections, an unsolvable puzzle (checking the board is restored after failure), the Inkala solution against its published ground truth, and a corrupted grid that `verify` must catch |

Everything is deterministic — candidates are tried in ascending order and
heuristic ties break toward the first cell — so the full output of both
runnable files is pinned with `expect` directives for `fable test`.

## Run it

From the repository root:

```sh
./target/release/fable demos/sudoku/main.fable          # solve the three puzzles (~0.6 s)
./target/release/fable demos/sudoku/main.fable --time   # same, plus wall-clock times
./target/release/fable test demos/sudoku                # golden tests (all four files)
```

## Sample output

```
== hard (Inkala 2010) — 21 clues ==

   puzzle                        solution
+-------+-------+-------+     +-------+-------+-------+
| 8 . . | . . . | . . . |     | 8 1 2 | 7 5 3 | 6 4 9 |
| . . 3 | 6 . . | . . . |     | 9 4 3 | 6 8 2 | 1 7 5 |
| . 7 . | . 9 . | 2 . . |     | 6 7 5 | 4 9 1 | 2 8 3 |
+-------+-------+-------+     +-------+-------+-------+
| . 5 . | . . 7 | . . . |     | 1 5 4 | 2 3 7 | 8 9 6 |
| . . . | . 4 5 | 7 . . |     | 3 6 9 | 8 4 5 | 7 2 1 |
| . . . | 1 . . | . 3 . |     | 2 8 7 | 1 6 9 | 5 3 4 |
+-------+-------+-------+     +-------+-------+-------+
| . . 1 | . . . | . 6 8 |     | 5 2 1 | 9 7 4 | 3 6 8 |
| . . 8 | 5 . . | . 1 . |     | 4 3 8 | 5 2 6 | 9 1 7 |
| . 9 . | . . . | 4 . . |     | 7 9 6 | 3 1 8 | 4 5 2 |
+-------+-------+-------+     +-------+-------+-------+
verified OK: assignments=10939 guesses=1850 backtracks=1837
```

The stats read as: `assignments` counts every digit placed (propagated
singles and guesses together), `guesses` counts speculative placements at
branch points, `backtracks` counts guesses undone after a contradiction.
The easy puzzle finishes at `guesses=0` — propagation alone solves it —
while the hard one needs 1850 guesses, which is exactly why it is hard.
