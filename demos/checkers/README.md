# checkers — an alpha-beta engine playing itself, in Fable

English draughts (8x8 checkers) with the full rule set — forced captures,
mandatory multi-jump chains, crowning, kings — driven by a negamax search
with alpha-beta pruning, a capture extension past the horizon, and a small
positional evaluation. The engine plays both sides of a complete game
deterministically: fixed depth, first-best tie-breaking, no randomness, so
every run produces the identical game (which the golden tests pin, node
counts and all).

About 450 lines of Fable in four files:

| File | What it does |
|------|--------------|
| `board.fable` | piece codes, board rendering, the move generator (forced captures, multi-jumps, crowning), and `apply`/`undo_move` so the search mutates one shared board instead of copying it |
| `engine.fable` | positional evaluation, negamax + alpha-beta with a capture extension, and `Stats` (whose `add` method overloads `+` for tallying) |
| `main.fable` | the self-play loop: draw detection (threefold repetition, 50 quiet plies), periodic board printing, final result and node statistics |
| `spec.fable` | golden tests for the rules and the search |

## Run it

From the repository root:

```sh
./target/release/fable demos/checkers/main.fable   # play the game (~15 s)
./target/release/fable test demos/checkers         # golden tests (all four files)
```

## Sample output

```
checkers self-play — negamax, depth 6 plus capture extension
black (b/B) moves up the board, white (w/W) moves down; captures are forced

  8    w   w   w   w
  7  w   w   w   w
  6    w   w   w   w
  5  .   .   .   .
  4    .   .   .   .
  3  b   b   b   b
  2    b   b   b   b
  1  b   b   b   b
     a b c d e f g h

  1. black c3-b4        eval     +0  nodes 4557
  2. white d6-c5        eval     -4  nodes 4505
  3. black b4xd6        eval     +4  nodes 1186
  4. white c7xe5        eval     -4  nodes 6016
...
 51. black e7-f8        eval   +110  nodes 3339  (crowned)
...
result:   draw — threefold repetition
material: black 2, white 2
search:   506738 nodes, 161291 beta cutoffs over 106 plies
```

The evaluation is printed from the mover's point of view in "centi-men"
(a man is worth 100). Both sides search to the same depth, so the game is
balanced: after trading down to two kings each, the engines shuffle and the
threefold-repetition rule calls the draw — an honest result for symmetric
self-play checkers.

## Notes

- **Search:** plain negamax over a single mutable board. `Board.apply`
  returns an `Undo` record (origin, destination, pre-crowning piece, and
  each victim's square and code) that `undo_move` replays in reverse —
  no board copies anywhere in the tree.
- **Forced captures:** the generator explores jump chains depth-first,
  mutating the board along the way so a piece can't be jumped twice, and
  emits a move only where the chain can't be extended (or where a man
  crowns, which ends the move by rule).
- **Capture extension:** when the nominal depth hits zero in a position
  where captures are forced, the search keeps going until the position is
  quiet, so a depth-6 search never mistakes the middle of a piece trade
  for a material swing.
- **Determinism:** move generation order is fixed (board scan order), and
  the root keeps the *first* best-scoring move, so the whole game — every
  move, every node count — is reproducible. `main.fable` carries
  `//? expect:` directives pinning the full game transcript.
