# png — a PNG encoder from scratch

A complete, valid PNG file built byte by byte in Socrates, with no
compression library and no compression: CRC-32 and Adler-32 implemented
from their definitions, and the deflate stream inside IDAT made of
*stored* (uncompressed) blocks — the trapdoor in RFC 1951 that makes a
fully spec-conformant PNG possible with nothing but framing.

```sh
./target/release/socrates demos/png/main.soc    # regenerate + verify out.png
./target/release/socrates test demos/png          # golden-run everything
```

## The layering

| File | What it holds |
|------|---------------|
| `bits.soc` | hex formatting only now (`to_hex`, `dump`) — the big-endian u32 / little-endian u16 accessors this used to also wrap were one-line pass-throughs to the v0.7 Bytes natives, so `std.png`/`std.zlib` (below) call those directly instead |
| [`std.crc`](../../std/crc.soc) | CRC-32 (reflected, poly `0xEDB88320`, 256-entry table built with eight shift-xor steps per byte) and Adler-32 — promoted from this demo's own `crc.soc`, unchanged |
| [`std.zlib`](../../std/zlib.soc) | an RFC 1950 stream wrapping raw bytes in an RFC 1951 *stored* block: `wrap` and its adversary `unwrap`, which re-checks header, LEN/NLEN, and the Adler trailer — promoted from this demo's own `zlib.soc`, renamed from `deflate_stored`/`inflate_stored` since no compression ever actually happens |
| `image.soc` | a 48x32 plasma in pure integer math — the sine is Bhaskara I's rational approximation, so the pixels (and therefore the PNG bytes) are identical on every machine |
| [`std.png`](../../std/png.soc) | chunk framing, the encoder (IHDR / IDAT / IEND), and a parser that recomputes every checksum in the file — promoted from this demo's own `png.soc`, unchanged except `encode` taking its stored-block size as an explicit parameter instead of a hardcoded module constant |
| `main.soc` | renders, encodes, writes `out.png`, re-reads it, and re-verifies everything |
| `spec.soc` | component tests: published CRC/Adler vectors, exact stored-block framing bytes, a corrupted-file drill |

## Worth seeing

- **The committed `out.png` is a build product and a golden test at
  once**: `main.soc` reads it *before* rewriting it and pins
  `committed out.png identical to fresh build: true` via structural
  `Bytes` equality.
- **Verification is external, not circular.** The IEND chunk's CRC must
  be the published constant `ae426082` (CRC-32 of the four bytes
  `IEND`), and `adler32("Wikipedia")` must be `11e60398` — numbers this
  program cannot invent. The whole file also cross-checks with any PNG
  tool: `python3 -c "import zlib; zlib.decompress(...)"` inflates the
  IDAT stream unchanged.
- **The corruption drill**: `spec.soc` flips one bit of pixel data in
  a 2x2 PNG and pins that exactly the IDAT CRC and the zlib Adler-32
  fail while IHDR and IEND stay green.
- **v0.7 at work**: `Bytes` (`push_u16le` writes deflate's LEN fields,
  `slice`/`concat` assemble chunks, `==` is deep), the bitwise operators
  everywhere a checksum or a nibble is extracted, `fs.read_bytes` /
  `fs.write_bytes` for the binary round-trip, and `strings.Builder` for
  the ASCII-preview accumulation.
