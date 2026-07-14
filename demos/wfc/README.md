# wfc — a wave-function-collapse texture generator in Fable

The simple-tile wave-function-collapse algorithm in about 350 lines of
Fable. From one small hand-drawn ASCII sample the demo learns which tiles
may sit next to which (and how often each tile occurs), then synthesises
larger textures that obey the same local rules:

1. every output cell starts in *superposition* — the set of all tiles,
   kept as a `Map[String, Bool]` used as a set (Fable has no set type);
2. the undecided cell with the lowest Shannon entropy (weighted by sample
   frequency, plus a pinch of random noise to break ties) is **collapsed**
   to a single weighted-random tile;
3. the change is **propagated** as arc consistency: neighbours drop
   candidates that no longer have support, and every cell that shrinks is
   re-queued to constrain its own neighbours;
4. a cell with no candidates left is a **contradiction** — the attempt is
   abandoned and generation restarts with the next seed.

Everything is driven by `math.seed`, so each seed is a reproducible
texture. Two tile sets ship with the demo: an *island* (sea → coast →
land → trees, which generates noisy archipelagos) and *pipes* (whose much
stiffer rules force every learned line to run edge to edge, generating
plumbing grids).

## Run it

From the repository root:

```sh
./target/release/fable demos/wfc/main.fable   # sample, tile set, two textures
./target/release/fable test demos/wfc         # golden tests (main + spec)
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
| `samples.fable` | the two hand-drawn training patterns, shared by main and spec |
| `rules.fable` | learning: tile inventory, weights, and the per-direction adjacency set (`"a:d:b"` keys in a map-as-set), plus the printable tile-set description |
| `wfc.fable` | the core loop: entropy bookkeeping, weighted collapse, arc-consistency propagation over a queue, restart-on-contradiction |
| `main.fable` | seeds, prints sample / learned rules / textures, and pins the full output with `//? expect:` directives |
| `spec.fable` | component golden tests: learned rules on a tiny sample, zero rule violations in generated output, same-seed determinism, and a provably impossible tile set that must exhaust its seed budget |

Notes on the algorithm:

- Entropy of a candidate set is `ln(Σw) − (Σ w·ln w)/Σw` — a cell whose
  remaining options are all rare tiles is "nearly decided" and collapses
  before a cell that could still be anything.
- Propagation is a breadth-first worklist (a list plus a read cursor).
  A neighbour keeps a candidate only while at least one of the source
  cell's candidates supports it in that direction.
- `generate(rules, w, h, seed, max_tries)` reseeds with `seed + attempt`
  on each contradiction and returns `None` only when the budget runs out —
  `spec.fable` exercises that path with rules learned from the single row
  `"ab"`, under which a 3-wide strip is unsatisfiable. (Since Fable v0.6
  `math.seed` scrambles its argument, so the adjacent seeds these retries
  use produce genuinely independent streams — under v0.5 they collided,
  which made a retry mostly replay the failed attempt.)
