# dungeon — a procedural roguelike dungeon generator in Fable

A classic roguelike map pipeline in about 300 lines of Fable: scatter random
non-overlapping rooms on a grid, chain them together with L-shaped corridors,
drop the player `@` in the first room and the treasure `$` in the room
farthest away, then let breadth-first search find the shortest route and draw
it onto the map with `*`. Generation is driven by `math.seed`, so every seed
is a reproducible dungeon; the demo prints two of them plus their stats.

## Run it

From the repository root:

```sh
./target/release/fable demos/dungeon/main.fable   # generate two dungeons
./target/release/fable test demos/dungeon         # golden tests (main + spec)
```

## Sample output

```
seed 7: 11 rooms, 34 steps from @ to $

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
| `dungeon.fable` | tiles, `Room` geometry (centers, padded intersection), the room-and-corridor generator, ASCII rendering |
| `path.fable` | breadth-first search: a list-plus-cursor queue, flat-encoded parent links, route reconstruction |
| `main.fable` | seeds the PRNG, generates, routes, marks the map, prints stats — and pins the full output with `//? expect:` directives |
| `spec.fable` | component golden tests: geometry, BFS on hand-drawn maps, generator invariants |

- **Rooms**: up to 80 candidate rooms are rolled per dungeon; a candidate is
  kept only if a one-cell-padded intersection test says it touches nothing
  already placed, so every room keeps a wall around itself.
- **Corridors**: each kept room is tunnelled to the previously placed one
  (horizontal-then-vertical or the reverse, by coin flip), which connects the
  whole dungeon by construction — `spec.fable` asserts the treasure is always
  reachable.
- **BFS**: the grid is flood-filled outward from `@`; the first time the
  search dequeues `$` the recorded parent links spell out a provably shortest
  route.
- **Rendering**: wall cells buried in solid rock (no open neighbor in any of
  the 8 directions) print as blanks, so only the dungeon's outline shows.

## Quirks found along the way

Fable v0.5's `math.seed(n)` ORed the seed with 1, so seeds `2k` and `2k + 1`
produced the same random stream (42 and 43 generated identical dungeons).
Fixed in v0.6: the seeder now scrambles the seed with SplitMix64, so adjacent
seeds give unrelated streams — and the spec's "different seed, different
dungeon" test now uses the tightest possible pair, 42 vs 43. The seeded
streams also changed wholesale in v0.6, which is why the pinned maps here
differ from the v0.5 ones.
