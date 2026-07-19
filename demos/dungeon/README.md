# dungeon — a procedural roguelike dungeon generator in Socrates

A classic roguelike map pipeline in about 390 lines of Socrates: scatter random
non-overlapping rooms on a grid, chain them together with L-shaped corridors,
drop the player `@` in the first room and the treasure `$` in the room
farthest away, then let breadth-first search find the shortest route and draw
it onto the map with `*`. A flood fill certifies full connectivity — every
carved tile reachable from `@`, not just the treasure. Generation is driven
by `math.seed`, so every seed is a reproducible dungeon; the demo prints two
of them plus their stats.

Modernized for v0.7: the BFS frontier is a `std.deque`, the visited flags
are a bit-packed grid (64 cells per `Int`, tested with `>> bit & 1`), the
flood fill's visited set is a `std.set` that doubles as the answer, the
renderer's "is anything open nearby" test is a bitwise 3×3 dilation
(shift-and-OR per row), and rows are assembled with `strings.Builder`. The
pinned maps did not change — the refactor left the `math.random` draw order
untouched, so the surviving goldens are the proof it is behavior-preserving.

## Run it

From the repository root:

```sh
./target/release/socrates demos/dungeon/main.soc   # generate two dungeons
./target/release/socrates test demos/dungeon         # golden tests (main + spec)
```

## Sample output

```
seed 7: 11 rooms, 34 steps from @ to $, all 318 tiles reachable

           ##########                ##########
           #........#   ###########  #........#
#######    #........#####.........#  #....$...#
#.....#    #......................## #....*...#
#.....#    #........#####..........# #####*.###
#.....#    ######.###   #..........#     #*.#
#.....#########.....#   ##########.####  #*.#
###.####.....##..@..#  ########.......####*.###
#.....##.....##..*..####.....##.......##***...#
#................************************.....#
#....................###.....##.......##......#
#.....##.....###..............................#
#.....######## #.....##########################
#######        #######
```

## How it works

| File | Role |
|------|------|
| `dungeon.soc` | tiles, `Room` geometry (centers, padded intersection), the room-and-corridor generator, bitmask-dilation ASCII rendering |
| `path.soc` | breadth-first search: `std.deque` frontier, bit-packed visited flags, flat-encoded parent links, route reconstruction; `reachable` flood fill over a `std.set` |
| `main.soc` | seeds the PRNG, generates, floods, routes, marks the map, prints stats — and pins the full output with `//? expect:` directives |
| `spec.soc` | component golden tests: geometry, BFS on hand-drawn maps, a bitset word-boundary crossing, flood coverage, generator invariants |

- **Rooms**: up to 80 candidate rooms are rolled per dungeon; a candidate is
  kept only if a one-cell-padded intersection test says it touches nothing
  already placed, so every room keeps a wall around itself.
- **Corridors**: each kept room is tunnelled to the previously placed one
  (horizontal-then-vertical or the reverse, by coin flip), which connects the
  whole dungeon by construction — `spec.soc` asserts every carved tile is
  reachable from the start, and `main.soc` prints the count.
- **BFS**: the grid is flood-filled outward from `@` with a `std.deque`
  frontier; visited cells are one bit each in a flat `List[Int]` (`>>` is
  arithmetic, so the test masks with `& 1` after the shift — a spec test
  drives the flags through bit 63 and across a word boundary on purpose).
  The first time the search dequeues `$` the recorded parent links spell out
  a provably shortest route.
- **Connectivity**: `path.reachable` floods with a `std.set` whose
  `insert -> Bool` return gates the search; the finished set *is* the
  answer, compared against the carved-tile count.
- **Rendering**: wall cells buried in solid rock (no open neighbor in any of
  the 8 directions) print as blanks. Each row's open cells pack into one
  `Int`; `bits | bits << 1 | bits >> 1` smears them horizontally and ORing
  three adjacent rows completes the 3×3 dilation, so the buried test is one
  `>> x & 1` per cell. Rows are built with `strings.Builder`.

## Quirks found along the way

Socrates v0.5's `math.seed(n)` ORed the seed with 1, so seeds `2k` and `2k + 1`
produced the same random stream (42 and 43 generated identical dungeons).
Fixed in v0.6: the seeder now scrambles the seed with SplitMix64, so adjacent
seeds give unrelated streams — and the spec's "different seed, different
dungeon" test now uses the tightest possible pair, 42 vs 43. The seeded
streams also changed wholesale in v0.6, which is why the pinned maps here
differ from the v0.5 ones.
