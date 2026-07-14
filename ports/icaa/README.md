# ICAA, ported to Fable

A CPU port of **ICAA — Isoline-Coverage Anti-Aliasing** by SkyeShark:
a novel single-frame, purely spatial post-process anti-aliasing technique
(structure-tensor-gated isoline crossings fit by weighted least squares,
resolved to closed-form box coverage, resynthesized in linear light).
Original: [SkyeShark/icaa-antialiasing](https://github.com/SkyeShark/icaa-antialiasing),
a three.js TSL/WebGPU node, MIT licensed — see [`LICENSE-upstream`](LICENSE-upstream)
and the repository `NOTICE`. Both presets (`quality`, `fast`) and all five
debug views are ported.

This is the first port through the [`jsl` translation layer](../README.md);
`icaa.fable` is structured section-for-section against the upstream
`ICAANode.js` so the two files can be read side by side.

## Run it

```sh
cargo build --release

# anti-alias a PPM (plain-text P3)
FABLE_PATH=ports ./target/release/fable ports/icaa/main.fable in.ppm out.ppm quality
FABLE_PATH=ports ./target/release/fable ports/icaa/main.fable in.ppm out.ppm fast

# debug views: 1=confidence 2=coverage 3=distance 4=orientation other>0=rms
FABLE_PATH=ports ./target/release/fable ports/icaa/main.fable in.ppm out.ppm quality debug 2

# trace every intermediate for one output pixel (the cross-debugging tool)
FABLE_PATH=ports ./target/release/fable ports/icaa/main.fable in.ppm /dev/null quality probe 64 47

# generate the deterministic test-scene suite
FABLE_PATH=ports ./target/release/fable ports/icaa/scenes.fable outdir/

# golden tests (this exact path — the CLI mains exit when run bare)
FABLE_PATH=ports ./target/release/fable test ports/icaa/spec.fable
```

## The receipts

The port is validated against an **independent transliteration of the same
TSL source**: [`reference/icaa-cpu.mjs`](reference/icaa-cpu.mjs), a
zero-dependency plain-JavaScript CPU implementation (node ≥ 18), written by
a separate author from the same shared contract (bilinear/clamp-to-edge
sampling with half-texel centers, three.js r178 sRGB constants, Rec. 601
luma, f64 everywhere, source expression order preserved — no algebraic
simplification).

Current state, re-verified by CI on every push:

- **18/18 pixel-exact**: 9 scenes × 2 presets, `max_diff=0` over every
  8-bit RGB component (`reference/compare.mjs`), plus 30 debug-view
  comparisons. An adversarial review round added 100+ more comparisons
  (noise fields, 0.02-slope edges, thin lines, rings, 1×1/8×1 images) —
  all exact.
- **Fast preset: bit-identical f64** — every probe intermediate matches the
  JS reference to the last bit. Quality preset matches everywhere except a
  ≤3e-16 relative residue in the final color, confined to the sRGB
  `pow` path (V8 `Math.pow` vs Rust `powf` are both allowed to be a few
  ulps off; neither has ever flipped an output byte).
- **Identity off-edge by construction**: flat fields, sub-threshold ramps,
  and checkerboards pass through byte-identical (pinned in `spec.fable`).
- The review round also caught a real contract violation — the layer's
  `norm()` used the reciprocal-sqrt form instead of divide-by-length,
  a 1-ulp difference that cascaded through the whole quality fit until
  fixed — which is exactly the class of bug the two-translation pattern
  exists to catch.

## Files

| File | What |
|------|------|
| `icaa.fable` | the algorithm, 1:1 with upstream `ICAANode.js` |
| `main.fable` | CLI (`in.ppm out.ppm quality|fast [debug N] [probe X Y]`) |
| `scenes.fable` | deterministic hard-aliased test scenes |
| `spec.fable` | golden tests (checksums + identity pins) |
| `reference/icaa-cpu.mjs` | independent plain-JS CPU reference |
| `reference/compare.mjs` | pixel-diff harness |
| `LICENSE-upstream` | upstream MIT license |
