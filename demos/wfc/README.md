# wfc — a wave-function-collapse texture generator in Socrates

The simple-tile wave-function-collapse algorithm in about 350 lines of
Socrates. From one small hand-drawn ASCII sample the demo learns which tiles
may sit next to which (and how often each tile occurs), then synthesises
larger textures that obey the same local rules:

1. every output cell starts in *superposition* — the set of all tiles.
   Tiles get small integer ids, so a tile set is an `Int` **bitmask**
   (v0.7 bitwise operators): membership is a shift-and-mask, set size is
   a popcount, intersection is `&`;
2. the undecided cell with the lowest Shannon entropy (weighted by sample
   frequency, plus a pinch of random noise to break ties) is **collapsed**
   to a single weighted-random tile (`1 << t`);
3. the change is **propagated** as arc consistency: a neighbour's new
   candidate set is one `&` against the union of its supporters'
   allow-masks, and every cell that shrinks is re-queued (a `std.deque`)
   to constrain its own neighbours;
4. a cell whose mask reaches `0` is a **contradiction** — the attempt is
   abandoned and generation restarts with the next seed.

Everything is driven by `math.seed`, so each seed is a reproducible
texture. Two tile sets ship with the demo: an *island* (sea → coast →
land → trees, which generates noisy archipelagos) and *pipes* (whose much
stiffer rules force every learned line to run edge to edge, generating
plumbing grids).

## Run it

From the repository root:

```sh
./target/release/socrates demos/wfc/main.soc   # sample, tile set, two textures
./target/release/socrates test demos/wfc         # golden tests (main + spec)
```

## Sample output

```
pipes sample 6x8
----------------
 |  |
 |  |
-+--+-
 |  |
 |  |
-+--+-
 |  |
 |  |

learned 4 tiles (weight = frequency in sample)
----------------------------------------------
' ' weight 24  up[ -  ] right[ |  ] down[ -  ] left[ |  ]
'|' weight 12  up[|+  ] right[    ] down[|+  ] left[    ]
'-' weight  8  up[    ] right[-+  ] down[    ] left[-+  ]
'+' weight  4  up[|   ] right[-   ] down[|   ] left[-   ]

generated 48x16 (seed 2026, attempt 1)
--------------------------------------
  |          |      | |  | |   |  |  | | |    |
  |          |      | |  | |   |  |  | | |    |
  |          |      | |  | |   |  |  | | |    |
--+----------+------+-+--+-+---+--+--+-+-+----+-
  |          |      | |  | |   |  |  | | |    |
  |          |      | |  | |   |  |  | | |    |
...
```

(The island texture, a 30x12 archipelago of `~ # . T`, comes first — run
the demo to see it.)

## How it works

| File | Role |
|------|------|
| `samples.soc` | the two hand-drawn training patterns, shared by main and spec |
| `rules.soc` | learning: tile inventory, weights, and the adjacency table — one bitmask per (tile, direction) in a flat `List[Int]` — plus `popcount` and the printable tile-set description |
| `wfc.soc` | the core loop: cached entropy bookkeeping, weighted collapse, arc-consistency propagation over a `std.deque`, restart-on-contradiction |
| `main.soc` | seeds, prints sample / learned rules / textures, and pins the full output with `//? expect:` directives |
| `spec.soc` | component golden tests: learned rules on a tiny sample, the bitmask rule table (all_mask, per-direction masks, popcount, forward/backward mask consistency), zero rule violations in generated output, same-seed determinism, and a provably impossible tile set that must exhaust its seed budget |

Notes on the algorithm:

- Entropy of a candidate set is `ln(Σw) − (Σ w·ln w)/Σw` — a cell whose
  remaining options are all rare tiles is "nearly decided" and collapses
  before a cell that could still be anything. Entropy is cached per cell
  and refreshed only when propagation actually shrinks a mask, so the
  selection scan is O(cells) instead of O(cells × tiles).
- Propagation is breadth-first over a `std.deque`. A neighbour's new
  candidate set is `cells[j] & support`, where `support` is the union
  (`|`) of `allow[s * 4 + d]` over the source cell's candidates `s` —
  the whole per-tile "does anything support you?" loop of the map-based
  version collapses into two bitwise operations.
- `generate(rules, w, h, seed, max_tries)` reseeds with `seed + attempt`
  on each contradiction and returns `None` only when the budget runs out —
  `spec.soc` exercises that path with rules learned from the single row
  `"ab"`, under which a 3-wide strip is unsatisfiable. (Since Socrates v0.6
  `math.seed` scrambles its argument, so the adjacent seeds these retries
  use produce genuinely independent streams — under v0.5 they collided,
  which made a retry mostly replay the failed attempt.)
- The v0.7 rewrite (map-as-set → bitmasks, entropy cache, `std.deque`
  queue, `strings.Builder` rendering) kept the RNG call order — and
  therefore every pinned texture — byte-identical, while making the
  main generation about 5x faster and an 80x40 island about 6.7x faster.
