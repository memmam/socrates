# plot — a function plotter in Fable

A small charting library and two demos built on it. Three series — a damped
sine, a damped cosine, and a cubic — are sampled into `Series` values,
framed by an affine data-to-pixel transform, and rendered as an SVG line
chart with gridlines, 1/2/5 "nice" ticks, a legend, and collision-dodged
direct labels at each line's end. A second SVG draws a hypotrochoid
("spirograph") as 75 hue-wheel strokes on a dark canvas. One series is also
printed as a terminal sparkline, so the demo has output even without a
browser.

## Run it

From the repo root:

```
./target/release/fable demos/plot/main.fable            # writes into demos/plot/
./target/release/fable demos/plot/main.fable some/dir   # writes into some/dir
./target/release/fable test demos/plot                  # golden tests
```

The generated `plot.svg` and `spirograph.svg` are committed as sample
output; running the demo regenerates them byte-for-byte (everything is
deterministic).

## Files

| File          | What it is                                                        |
|---------------|-------------------------------------------------------------------|
| `svg.fable`   | a tiny SVG builder: a `Doc` collects elements, `render()` joins them |
| `chart.fable` | series sampling, the `Frame` transform, nice ticks, the line-chart renderer, sparklines |
| `main.fable`  | the three series, the spirograph, CLI glue, and the golden `//? expect:` output |
| `checks.fable`| unit-style golden tests against the modules' public API           |
| `plot.svg`, `spirograph.svg` | the committed sample output                        |

## Sample output

```
plot — a function plotter written in Fable

sin(x)·e^(-x/4)  ▃▅▆▇▇████▇▇▆▅▄▄▃▂▂▂▁▁▁▁▁▁▂▂▂▃▃▃▄▄▄▄▄▄▄▄▄▄▄▄▃▃▃▃▃
                 48 samples on [0, 10], minimum -0.318 near x = 4.468

wrote demos/plot/plot.svg (8809 bytes, 3 series x 120 samples)
wrote demos/plot/spirograph.svg (22839 bytes, 901 points)
```

## Language features on display

- multi-file structure with `import`, `pub`, and impl-block methods
- closures as data: `chart.sample` takes any `fn(Float) -> Float`
- string interpolation building the whole SVG document (`join`, `map`)
- `Result` + the `?` operator plumbing `fs.write` failures to one handler
- tuples for points, `zip`/`enumerate`/`fold`/`sort_by` on lists, destructured
  right in the loop header (`for (i, s) in …`, v0.6)
- v0.6 stdlib: `math.log10` picks tick magnitudes, `to_fixed(3)` prints the
  sparkline caption
- golden tests via `//? expect:` directives (`fable test demos/plot`)
