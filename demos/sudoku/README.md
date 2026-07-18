# sudoku — constraint propagation + backtracking search, on 9-bit masks

A complete sudoku solver whose candidate sets are **9-bit Int masks**
(v0.7 bitwise operators): bit d-1 stands for digit d, so "any of 1..9" is
`0b111111111 = 511` and the whole constraint algebra is arithmetic —
union `|`, intersection `&`, complement `^ 511`, membership one shift and
one `&`. The board keeps one used-digit mask per row, column, and box;
the legal digits for a cell are `(row | col | box) ^ 511`, computed fresh
in O(1) whenever asked.

Naked-singles propagation fills every cell whose mask has exactly one bit
(`popcount(mask) == 1` — popcount is a one-line `count_ones` wrapper),
looping to a fixpoint; when logic alone stalls, a depth-first search
guesses at the **most-constrained cell** (fewest mask bits first) and
backtracks on contradiction (`mask == 0`). Guesses peel the mask
lowest-bit-first — ascending digit order — so the search is fully
deterministic. The board is one shared mutable grid; placements are
logged on a trail so a failed branch unwinds exactly what it added.

Three inline puzzles show the spread: an easy one that propagation solves
outright (zero guesses), Arto Inkala's 2010 "world's hardest sudoku", and
a 17-clue puzzle from Gordon Royle's minimal-sudoku collection (17 clues
is the proven minimum for a unique solution). Every answer is re-checked
by an independent verifier — all 27 units and every original given — and
the run prints search statistics per puzzle.

About 650 lines of Socrates in four files:

| File | What it does |
|------|--------------|
| `board.soc` | the 9-bit mask kit (`digit_bit`, `has`, `popcount`, `lowest_digit`, `digits_of`), the `Board` struct (81 cells + one used-mask per row/column/box), parsing with validation, Builder-based pretty-printing, and the independent `verify` (each unit folds to a mask that must equal 511) |
| `solver.soc` | naked-singles propagation, the MRV cell chooser, trail-based backtracking that iterates guesses by peeling mask bits, and `Stats` (assignments / guesses / backtracks) |
| `main.soc` | the three puzzles, side-by-side puzzle/solution rendering, `?`-based error plumbing, an optional `--time` flag |
| `spec.soc` | golden tests: mask-kit self-tests (full mask = 511, digit-bit round trips, `&`/`\|`/`^` set algebra, the precedence the code leans on, arithmetic `>>`), parser rejections, an unsolvable puzzle (checking the board is restored after failure), the Inkala solution against its published ground truth, a candidate-elimination check on a real grid, and a corrupted grid that `verify` must catch |

Everything is deterministic — candidates are tried in ascending digit
order and heuristic ties break toward the first cell — so the full output
of both runnable files is pinned with `expect` directives for
`socrates test`. The mask representation reproduces the exact same search
as the v0.5 list-of-candidates version (same masks, same order), so the
pinned solve narratives — down to `assignments=10939 guesses=1850
backtracks=1837` on the Inkala — are unchanged, while the whole run got
about 2x faster (no per-cell candidate lists to allocate).

## Run it

From the repository root:

```sh
./target/release/socrates demos/sudoku/main.soc          # solve the three puzzles (~0.3 s)
./target/release/socrates demos/sudoku/main.soc --time   # same, plus wall-clock times
./target/release/socrates test demos/sudoku                # golden tests (all four files)
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

## The mask idioms, if you need them elsewhere

Everything the solver knows about bit fiddling is in the ~40-line kit at
the top of `board.soc`:

```soc
pub let full_mask = 0b111111111;                   // digits 1..9
pub fn digit_bit(d: Int) -> Int { 1 << d - 1 }     // `-` binds tighter than `<<`
pub fn has(mask: Int, d: Int) -> Bool { mask >> d - 1 & 1 == 1 }
```

Precedence is Rust's: arithmetic > shifts > `&` > `^` > `|` > comparisons,
so none of the expressions above need parentheses. One caution carried in
the code comments: Socrates's `>>` is **arithmetic** (sign-extending) — 9-bit
masks never touch the sign bit so it costs nothing here, but a full-width
bitboard must re-mask after every right shift. Mask updates use v0.8's
compound bitwise assignment (`self.row_used[r] |= bit`, `^=` to undo).
