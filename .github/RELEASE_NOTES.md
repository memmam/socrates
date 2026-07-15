Fable is a statically-typed, garbage-collected programming language — ADTs,
exhaustive pattern matching, closures, generics, modules, a test runner, a
language server, a REPL, and an embedded standard library — implemented from
scratch in about 21,000 lines of Rust with **zero dependencies** (the one
optional dependency, wgpu, sits behind the `gpu` cargo feature; the default
build's `cargo tree` is a single line).

v0.7 is the **infrastructure release**: the layer that turns Fable from a
capable interpreter into something you build systems with. Bit-exact binary
data, true OS-thread parallelism, a native FFT, and — new in this release —
a way to ship a whole program as one self-contained binary. Two independent
codebases were ported onto it and cross-validated to numeric and pixel
equality against their originals.

Highlights of v0.7 over v0.6 (full list in
[`CHANGELOG.md`](https://github.com/memmam/fable/blob/main/CHANGELOG.md)):

- **`fable build`** — pack a program into one self-contained executable. Its
  modules, data files, and worker `.fable`s are stapled onto a copy of the
  interpreter as a dependency-free appended payload; the binary unpacks itself
  at startup and runs, byte-identical to the source run. Cross-target by
  design, which is how this release ships the **demo zoo** (below).
- **`Bytes`** — packed byte buffers with checked accessors, little- and
  big-endian multi-byte pushers and readers, `slice`/`concat`, UTF-8 bridging,
  structural equality, and map-key support; plus `fs.read_bytes` /
  `fs.write_bytes` for binary I/O.
- **Bitwise operators on `Int`** — `& | ^ << >>` (Rust precedence; `>>`
  arithmetic; out-of-range shifts panic), with intrinsics `count_ones`,
  `leading_zeros`, `trailing_zeros`, `ushr`, `rotate_left`, `rotate_right`,
  and `to_hex`.
- **`fft` namespace** — native `fft` / `ifft` / `rfft` over split-complex
  signals, any length ≥ 1 in O(n log n) (radix-2 for powers of two, Bluestein
  otherwise), numpy conventions, cross-checked against numpy in CI at 1e-9.
- **Workers** — `worker.spawn(file, args)` runs a program as an *isolate*: its
  own VM, heap, and GC on its own OS thread, joined to the parent only by
  string channels. Compile errors surface synchronously; a worker's panic
  comes back as `Err` from `join`.
- **`gpu` namespace (experimental, feature-gated)** — dispatch WGSL compute
  shaders over `Bytes` I/O, implemented with wgpu behind `--features gpu`.
  Without the feature it still typechecks and degrades gracefully, so the
  default build stays zero-dependency.
- **`std` collections** — `std.set`, `std.deque`, `std.lists`
  (`min_by`/`max_by`/`sort_by`, sums, fills), and `strings.Builder` for O(n)
  string accumulation.
- **A line-width-aware formatter** — `fable fmt` fits constructs on one line
  when they fit in 100 columns (`--width N` to change it) and breaks them the
  way an author would when they don't; idempotent and behavior-preserving.
- **The efficiency pass** — a measured, benchmark-gated optimization sweep
  (method and numbers in
  [`bench/RESULTS.md`](https://github.com/memmam/fable/blob/main/bench/RESULTS.md)):
  checkers −15%, lisp −20%, string building −55%, map ops −37%, and −67% on
  the heaviest demo under GC stress — all with byte-identical golden output.

Everything observable is pinned: 294 golden spec tests, 122 executable book
snippets, and 71 demo golden tests, the whole suite green under
`FABLE_GC_STRESS=1`. Two ports validated end-to-end (ICAA 18/18 pixel-exact;
claudewave 32/32 battery, 29 bit-exact).

## The demo zoo

Every one of the seventeen [`demos/`](https://github.com/memmam/fable/tree/main/demos)
— a Lisp, a spreadsheet, a backtracking regex engine, checkers with an
alpha-beta engine, a from-scratch PNG encoder, a chiptune renderer, a
parallel Mandelbrot, and ten more — ships as a **self-contained binary**: no
`fable`, no source, one file you run. They are attached as
`fable-demozoo-v0.7.0-<target>.tar.gz` for five desktop targets:

- `x86_64-linux`, `aarch64-linux`
- `x86_64-windows`, `aarch64-windows`
- `aarch64-macos` (Apple Silicon)

Unpack with `tar -xf` (built in on Windows 10+ too) and run any animal in the
zoo. On macOS the payload rides in a Mach-O section (appending it would break
code signing); the binaries are ad-hoc signed, so a downloaded copy needs
Gatekeeper cleared once — `xattr -d com.apple.quarantine ./<demo>` — until a
notarized build lands.

## Getting started

Unpack the attached interpreter for your platform (or `cargo build --release`
from source on anything Rust supports), then:

```sh
fable examples/mandelbrot.fable
fable demos/checkers/main.fable      # watch it play itself
fable test tests/spec demos          # run the whole golden suite
fable build demos/lisp -o lisp && ./lisp   # or make your own standalone binary
fable repl
```

Licensed under the Apache License 2.0 — see `LICENSE` and `NOTICE`.
