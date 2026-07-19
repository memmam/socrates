# The Socrates demos

Eighteen programs, each a self-contained showcase of the language doing real
work. Every demo is deterministic, pins its complete output with golden
`//?` directives, and passes under GC stress:

```sh
cargo build --release
./target/release/socrates demos/lisp/main.soc      # run one
# glcube's three mains need a live GL/Metal/Vulkan window (CI runs them in
# the windowing jobs); everything else, cube.soc/spec.soc included:
shopt -s extglob
./target/release/socrates test demos/!(glcube)/ demos/glcube/cube.soc demos/glcube/spec.soc
SOCRATES_GC_STRESS=1 ./target/release/socrates test demos/!(glcube)/ demos/glcube/cube.soc demos/glcube/spec.soc
```

| Demo | What it does | Worth seeing |
|------|--------------|--------------|
| [`lisp/`](lisp/) | A mini-Lisp: reader, evaluator, and six sample programs (factorial, fib, a Lisp-level `map`, closures, a 100k-iteration loop, parallel `let`). | Socrates's TCO reaches *through* the interpreter — the tail-recursive Lisp loop runs in constant stack. `try()` turns VM panics into Lisp error values; a `std.set` of reserved words drives special-form dispatch and `strings.Builder` threads through the recursive printer. |
| [`spreadsheet/`](spreadsheet/) | Formulas with a Pratt parser, dependency-driven evaluation, memoization, and spreadsheet-faithful error values; min/max/avg torture-tested on empty, words-only, and error-poisoned ranges. | Cycle detection is a single `std.set` insert — `insert()` returning `false` *is* the `#CYCLE!` (which still *heals* when the cycle is edited away). Empty ranges split by identity: `sum`/`count` give 0, `avg`/`min`/`max` give `#VALUE!` via `std.lists` Options. |
| [`regex/`](regex/) | A backtracking regex engine: literals, classes, anchors, `* + ? \|` and v0.7 `{m,n}`, groups, escapes. 87 self-checking tests + grep mode underlining every match on a line. | Character classes compile to 256-bit bitmaps in a `Bytes` buffer — membership is `bits.get(c >> 3) >> (c & 7) & 1 == 1` — and `{m,n}` desugars to seq/opt/star so the CPS matcher stayed untouched. |
| [`dungeon/`](dungeon/) | A seeded roguelike dungeon generator: rooms, L-corridors, BFS shortest path drawn onto the ASCII map, plus a flood-fill certificate that every carved tile is reachable. | Modernized for v0.7 with the old pins as proof: `std.deque` frontier, bit-packed visited flags (a spec test crosses bit 63 and a word boundary on purpose), shift-OR 3x3 dilation in the renderer — and the maps came out byte-identical. |
| [`mdsite/`](mdsite/) | A static site generator: markdown → templated HTML site, three sample pages, build report with a regeneration check. | All string assembly on `strings.Builder` (v0.7); `std.set` guards slug collisions; `fs.read_bytes` + structural `Bytes` `==` pins the committed `out/` byte-for-byte against a fresh build. |
| [`csvql/`](csvql/) | A query language over CSV: `select country, count, min city, max city group by country order by count desc`. | Group-by buckets keyed by enum values in a Map (structural hashing + insertion order = deterministic reports); v0.7 `min`/`max` aggregate any column — `std.lists.min_by`/`max_by` over `Val.cmp` orders text cells too — and the report renders through one `strings.Builder`. Every malformed query degrades to one tidy error line. |
| [`checkers/`](checkers/) | Full English draughts (forced captures, multi-jumps, kings) with a negamax alpha-beta engine; draws detected via 64-bit Zobrist hashes in `std.set`s. | A complete 106-ply self-play game — every move, eval, and node count pinned as ~200 golden lines — replayed byte-identically after the v0.7 rewrite: xorshift64 keys built from `^`/`<<`/`ushr` (logical, not `>>`, which would smear the sign bit), threefold repetition called by a two-set `insert` gate. |
| [`plot/`](plot/) | A function plotter: SVG line charts with 1/2/5 nice ticks and collision-dodged labels, a 75-stroke spirograph, and a two-tone `fft.rfft` magnitude spectrum as a stem chart, plus terminal sparklines. | Regenerates all three committed SVGs byte-for-byte — and *pins that claim* as golden lines (`fs.read_bytes` + structural `Bytes` equality). Every document is assembled through one `strings.Builder`, in `std.svg`. |
| [`sudoku/`](sudoku/) | Naked-singles propagation + most-constrained-cell backtracking over three classic puzzles (including Inkala's "hardest"), with candidate sets as 9-bit Int masks (v0.7 bitwise). | Set algebra as arithmetic: candidates are `(row \| col \| box) ^ 511`, naked singles are `popcount(mask) == 1`, guesses peel the mask lowest-bit-first. Byte-identical solve narrative to the list version at ~2x the speed; the independent verifier still catches a deliberately corrupted board. |
| [`wfc/`](wfc/) | Wave-function collapse: learns tile adjacency from ASCII samples, generates new textures by entropy-driven constraint propagation. | The spec pins the *contract*, not just output: zero adjacency violations, same-seed determinism, and a provably impossible tile set that must exhaust its seed budget. |
| [`parmandel/`](parmandel/) | The Mandelbrot set rendered by four worker isolates (v0.7), each band on its own OS thread in its own VM, streaming rows back over string channels. | The output pins exactly despite true parallelism: per-worker message order is FIFO and assembly drains band by band — determinism by protocol, not by luck. |
| [`synthwave/`](synthwave/) | A chiptune track renderer: square + triangle + LFSR-noise voices with ADSR envelopes, two bars of 12/8 at 8000 Hz, packed into the committed, playable `track.wav` via the v0.7 Bytes LE pushers. | The tune proves itself twice: the rebuild must equal the committed WAV byte-for-byte (no `math.sin` in the signal path — phase accumulators and bitwise ops only), and `fft.rfft` must re-detect each probe note on its exact intended bin, 3rd harmonic as runner-up. |
| [`png/`](png/) | A PNG encoder over `std.crc`/`std.zlib`/`std.png`'s from-scratch CRC-32/Adler-32/*stored*-deflate primitives, plus the demo's own 48x32 integer-math plasma — a fully valid PNG with no compression at all. | The committed `out.png` is pinned byte-identical to a fresh build via structural `Bytes ==`, the parser re-verifies every checksum in its own file, and the IEND chunk's CRC must equal the published constant `ae426082` — a number the program cannot invent. |
| [`bloom/`](bloom/) | A Bloom filter over `Bytes`: FNV-1a and a xorshift* mixer, hand-built from the v0.7 bitwise operators, double-hash a 500-word generated corpus into 512 bytes — then a `std.set` oracle grades every answer. | Zero false negatives and an exact pinned false-positive count (38/2000) landing on the textbook `(1-e^(-kn/m))^k`; a 32x32 multiply in 16-bit halves because Int overflow panics; the committed `filter.bin` regenerates byte-identically. |
| [`spectra/`](spectra/) | A chord analyzer on `fft.rfft`: just-intonation chords synthesized onto exact integer bins, then re-identified from the spectrum alone — ASCII bar spectrograms, `max_by` + a set-marked top-k, and gcd-reduced ratios naming major/minor/fifth. | One-second windows make bin k exactly k Hz, so ~115 lines of spectral analysis pin exactly; Parseval, the `ifft(fft(x))` round trip, and a naive-DFT cross-check all hold at 1e-9 — Bluestein path included (n = 600 and 12). |
| [`swarm/`](swarm/) | A worker-pool job scheduler: three isolates crunch Collatz and prime-count jobs from a `std.deque` queue over a `std.json` protocol — static assignment, dynamic feed-on-return balancing, and panic isolation. | A fragile worker's panic comes back as `Err` from `join` and its job JSON re-runs on a fresh isolate; the dynamic section pins only schedule-independent facts, so a smarter scheduler could drop in without re-pinning a line. |
| [`reversi/`](reversi/) | Othello on two Int bitboards: shift-and-propagate move generation in 8 masked directions, flood-and-confirm flips, `count_ones` popcount, and a complete greedy self-play game pinned move for move. | Every classic bit trick had to be re-derived for signed-64-with-panicking-overflow — `bits.soc` documents each trap (`>>` is arithmetic; `x & -x` panics on bit 63). The move generator is proven by pinned perft(1..6) = 4/12/56/244/1396/8200. |
| [`glcube/`](glcube/) | A unit cube spinning about its vertical axis, rendered through the `gfx.*` draw-call surface on the `window` namespace, with the MVP matrix built from `std.glm`'s `Mat4` — the same geometry and shader logic run three times over, against OpenGL, Metal, and Vulkan. | Pixel spot-checks, not framebuffer hashes: two frames read back one solid-color pixel each (front face at rest, left face after a quarter turn), so the golden pins hold regardless of driver-level antialiasing. `main_metal.soc` and `main_vulkan.soc` reuse `cube.soc` unchanged and pin **byte-identical** output to `main.soc` — the same program rendering the same pixels on three graphics APIs. Needs a live GL/Metal/Vulkan window; `cube.soc`/`spec.soc` need none. |

## The demo zoo — download and run

Every demo also ships as a **self-contained binary** in each release: no
`socrates`, no source tree, no runtime — one file you run. They are built with
`socrates build`, which staples a program (its modules, its data files, and the
worker `.soc` files it spawns) onto a copy of the interpreter; on launch the
binary unpacks itself into a scratch directory and runs, so its output is
byte-for-byte what `socrates demos/<name>/main.soc` prints.

```sh
socrates build demos/lisp -o lisp     # build one yourself
./lisp                             # run it anywhere
```

The release carries the whole zoo cross-compiled for five desktop targets —
`x86_64` and `aarch64` Linux, `x86_64` and `aarch64` Windows, and Apple
Silicon macOS — as `socrates-demozoo-<version>-<target>.tar.gz`. Extract with
`tar -xf` (built in on Windows 10+ as well) and run any animal in the zoo. On
Linux and Windows the payload is appended to the interpreter; on macOS — where
appending past the Mach-O `__LINKEDIT` would break code signing — it is linked
in as a `__DATA,__socrateszoo` section instead. The macOS binaries are ad-hoc
signed, so a downloaded copy needs `xattr -d com.apple.quarantine ./<demo>`
once until a notarized build lands.

## Where they came from

The first ten demos were written against **v0.5** by ten independent
authors with a double brief: make something interesting, and surface every
papercut. Each was then verified by a separate reviewer following only its
README. Their issue reports — deduplicated, triaged, and answered — are in
[`NOTES.md`](NOTES.md); the fixes they drove became v0.6, and the demos
were then modernized to use what they'd asked for.

The same process ran again for **v0.7**: six new demos built on the
infrastructure release (Bytes, FFT, workers, bitwise, the std collections)
plus a modernization pass over all eleven existing ones, seventeen authors
and seventeen adversarial verifiers in all. (Correction: that's actually
seven new — `parmandel` included — and ten existing; see `NOTES.md`
§ "The v0.7 round" for the detail.) That round's triage is in
`NOTES.md` § "The v0.7 round", and its distilled house rules — best
practices as designed to now — are [`STYLE.md`](STYLE.md).

`glcube/`, the eighteenth demo, arrived separately with v0.8's native
window/`gfx` work rather than through either field-test round — its own
README documents its pixel-spot-check verification convention.
