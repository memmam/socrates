# parmandel — the Mandelbrot set on four worker isolates

A 60×24 ASCII Mandelbrot rendered in parallel. The image splits into
four horizontal bands; each band goes to a `worker.spawn`ed isolate —
its own VM, heap, and GC on its own OS thread — which streams finished
rows back to the parent over the string channel.

The pinned output is byte-identical no matter how the threads
interleave, and that is a property of the **protocol**, not luck:

1. Workers never print. All output flows through `worker.send`, and
   within one worker messages arrive in send order (the channel is
   FIFO).
2. The parent drains the fleet band by band, in spawn order: `recv`
   until `None` (worker finished, channel drained), then `join` — which
   also surfaces a worker panic as `Err` instead of losing it.
3. All parameters ride in as spawn args (`os.args()` inside the
   worker), so each isolate is a pure function of its arguments.

`row_worker.soc` doubles as its own golden test: guarded by
`worker.is_worker()`, a standalone run does nothing and prints nothing.

## Run it

From the repository root:

```
./target/release/socrates demos/parmandel/main.soc   # render
./target/release/socrates test demos/parmandel         # golden tests
```

## Files

| File               | What it is                                                     |
|--------------------|----------------------------------------------------------------|
| `main.soc`       | spawns the fleet, drains rows band by band, joins each worker  |
| `row_worker.soc` | the isolate: escape-time iteration, palette lookup, one row per `send` |

## v0.7 features on display

- workers: `worker.spawn(file, args)`, handle `recv`/`join`, child-side
  `worker.send`/`worker.is_worker`, `os.args()` as spawn args
- `strings.Builder` accumulating each row — one builder `clear`ed per
  row, `push_char` of the palette's `code_at` code per pixel, so a row
  is O(width) instead of the O(width²) of `+=` per character (and no
  one-character string is allocated per pixel)

## Determinism notes

The escape-time loop is plain `f64 * + -` arithmetic and integer
comparisons — no libm — so iteration counts are exact and the palette
characters pin safely on every machine.
