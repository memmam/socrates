# bloom — a Bloom filter over Bytes, measured against std.set

A Bloom filter built from v0.7's raw materials — m bits packed in a
`Bytes` buffer, two hand-written 32-bit hashes combined by double
hashing — and then *measured* instead of trusted: a 500-word generated
corpus goes in, and the demo pins zero false negatives, the exact
false-positive count over 2000 disjoint probes (with `std.set` as the
oracle), and the textbook estimate `(1 - e^(-kn/m))^k` alongside the
observed rate.

```sh
./target/release/socrates demos/bloom/main.soc   # regenerate + verify filter.bin
./target/release/socrates test demos/bloom         # golden-run everything
```

## The layering

| File | What it holds |
|------|---------------|
| `hash.soc` | FNV-1a 32 (xor, multiply, mask), `mul32` (32x32 multiply mod 2^32 that cannot trip the overflow panic), the xorshift* mixer, popcount over the `count_ones` intrinsic, hex formatting |
| `words.soc` | the corpus generator: a C-standard LCG written out in plain integer arithmetic — no `math.random`, so the corpus is identical in every release forever |
| `bloom.soc` | the filter: bit i at byte `i >> 3` under mask `1 << (i & 7)`, double-hashed probes `h1 + i*h2` with the stride forced odd, popcount/fill, a 14-byte serialization format, and the theoretical estimates |
| `main.soc` | corpus in, contract out: false negatives (0), exact false positives vs the `std.set` oracle, theory alongside, a geometry sweep, bit-level introspection, and the committed-artifact round-trip |
| `spec.soc` | component tests: published FNV-1a vectors, `mul32` wrap identities, avalanche counts, popcount vectors, bit set/get, serialization down to its exact bytes plus its failure paths |
| `guardrails.soc` | pins the panic: `bloom.new(4095, 4)` is refused because the index mask `& m - 1` needs a power of two |
| `filter.bin` | the committed artifact: 14-byte header + 512 filter bytes; `main.soc` reads it *before* rewriting it and pins byte-identity with the fresh build |

## Worth seeing

- **The filter is graded by an oracle, not by itself.** Every probe is
  checked against `std.set` ground truth word by word; the false-positive
  count (38/2000) is exact and pinned, and lands within noise of the
  textbook `(1 - e^(-kn/m))^k` = 0.0196 — evidence the two hand-rolled
  hashes are actually independent.
- **External truth**: FNV-1a's published vectors (`"" -> 811c9dc5`,
  `"foobar" -> bf9cf968`), `mul32(0xFFFFFFFF, 0xFFFFFFFF) = 1` (an
  algebraic identity), and popcount summing to exactly 1024 over all 256
  bytes.
- **The overflow panic shapes the code.** Socrates's Int panics on overflow,
  so 32-bit hashing masks with `& 0xFFFFFFFF` after every step and a full
  32x32 multiply wraps in 64 bits and masks (`mul32`, one line over
  `wrapping_mul`). The arithmetic `>>` hazard is pinned on purpose:
  `-8 >> 1 = -4`, mask after shifting.
- **A geometry sweep on the same corpus**: 1024 bits drown (fp rate 0.48),
  16384 bits with k=8 let exactly one probe through (rate 0.0005) — the
  whole time/space trade-off in four pinned lines.
- **v0.7 at work**: `Bytes` as the bit store (`get`/`set`, `slice`,
  `concat`, structural `==`), the little-endian pushers writing the
  serialization header, bitwise operators in every hash and mask,
  `std.set` as the oracle, `std.lists` for the corpus stats,
  `strings.Builder` for hex/binary rendering, `fs.read_bytes`/`write_bytes`
  for the committed artifact.
