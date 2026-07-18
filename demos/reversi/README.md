# reversi — Othello on Int bitboards, in Socrates

An Othello (reversi) engine whose entire board state is two 64-bit `Int`s —
one bitboard per color, bit *i* = square *i* with a1 = 0 and h8 = 63. Move
generation is the classic shift-and-propagate flood in 8 directions with
file-edge masks; flips are found by flooding from the placed disc and
confirming a friendly disc caps the run; scores are popcounts. Two copies
of the same greedy engine (max flips, lowest-square tie-break) play a
complete deterministic game whose transcript is pinned move for move, and
the move generator is validated perft-style against the known Othello
node counts.

About 600 lines of Socrates in four files (a fifth of that the pinned
game transcript):

| File | What it does |
|------|--------------|
| `bits.soc` | the 64-bit toolbox: one-line delegations to the `ushr`/`count_ones`/`trailing_zeros` intrinsics plus bitboard iteration — the documented reference for the signed-64 traps each intrinsic retires |
| `board.soc` | edge masks, the 8-direction table, shift-and-propagate move generation, flood-and-confirm flips, `apply`, coordinate names, and a `strings.Builder` renderer |
| `main.soc` | greedy self-play from the standard opening to the full board, with the board printed every 12 plies and every move, score, and pass pinned |
| `spec.soc` | golden tests: shift/popcount/ctz self-tests, edge-mask and corner probes, opening movegen, flip confirmation, and perft(1..6) = 4 / 12 / 56 / 244 / 1396 / 8200 |

## Run it

From the repository root:

```sh
./target/release/socrates demos/reversi/main.soc   # play the game (instant)
./target/release/socrates test demos/reversi         # golden tests (all four files)
```

## Sample output

```
  8 . . . . . . . .
  7 . . . . . . . .
  6 . . . . + . . .
  5 . . . X O + . .
  4 . . + O X . . .
  3 . . . + . . . .
  2 . . . . . . . .
  1 . . . . . . . .
    a b c d e f g h

  1. black d3  flips  1  score 4-1
  2. white c3  flips  1  score 3-3
...
 60. white a7  flips  1  score 19-45

     black passes
     white passes
final score:  black 19, white 45 — white wins
```

Greed is a famously bad Othello strategy — maximizing flips early hands
your opponent the stable edges — and the pinned game shows it: black leads
20–5 at move 21 and still loses 19–45.

## Notes

- **Movegen** (`board.legal_moves`): for each direction, flood the mover's
  discs through runs of opponent discs (five propagation steps cover the
  longest possible run), then step once more onto an empty square. The
  known perft values from the opening — 4, 12, 56, 244, 1396, 8200 — pin
  its correctness in `spec.soc`.
- **Wraparound**: every direction carries a post-shift mask. A disc on
  file h shifted east must not reappear on file a of the next rank, so
  east-ish directions mask with `not_file_a`, west-ish with `not_file_h`;
  pure north/south cannot wrap. `0x8080808080808080` (file h) has bit 63
  set — unwritable as a literal until v0.8's full-width hex, so it is now
  written directly (the derived `file_a << 7` lives in git history).
- **Signed-64 traps**: Socrates's `>>` is arithmetic and overflow panics, so
  a bitboard with h8 occupied breaks three classic idioms — right shifts
  smear the sign bit (the `ushr` intrinsic zero-fills), and both
  `x & (x - 1)` and `x & -x` panic on the bit-63-only value (bit
  iteration clears with `x ^ bit(ctz(x))` instead). The hand-rolled SWAR
  popcount this demo once carried even had to count bit 63 separately —
  its first halving step could carry into the sign bit; that version
  lives in git history, and counting is now the `count_ones` intrinsic.
  Each trap is a comment in `bits.soc` and a pinned test in
  `spec.soc`.
- **Determinism**: `bits.squares` yields moves in ascending square order
  and `std.lists.max_by_key` keeps the first winner on ties, so "greedy
  max-flips, lowest square wins" needs no explicit tie-break code at all.
