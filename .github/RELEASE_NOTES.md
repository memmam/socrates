Fable is a statically-typed, garbage-collected programming language — ADTs,
exhaustive pattern matching, closures, generics, modules, a test runner, a
language server, a REPL, and an embedded standard library — implemented from
scratch in about 22,500 lines of Rust with **zero dependencies** (the one
optional dependency, wgpu, sits behind the `gpu` cargo feature; the default
build's `cargo tree` is a single line).

v0.8 works straight through the feature-request queue the v0.7 demo round
left behind: seventeen independent demo authors, seventeen adversarial
verifiers, and a ranked, deduplicated list of every wall they hit that v0.7
didn't already remove (`demos/NOTES.md`). This release clears it — mostly
ergonomics and a few genuine gaps, no new subsystems, every item traceable
to a real program that wanted it.

Highlights of v0.8 over v0.7 (full list in
[`CHANGELOG.md`](https://github.com/memmam/fable/blob/main/CHANGELOG.md)):

- **`if let` / `while let`** — test a single pattern without a full `match`.
  Both are pure parser sugar, desugared fully to `match`/`while` at parse
  time, so the checker and compiler need no special cases at all.
- **Bitwise compound assignment** — `&= |= ^= <<= >>=`, matching the
  arithmetic set (`+=` and friends).
- **64-bit hex/binary literals** — `0x8080808080808080` and other patterns
  with bit 63 set are now writable (previously only reachable by shifting);
  `String.parse_hex()` is `to_hex()`'s inverse.
- **`Bytes` 64-bit accessors** (`push`/`read_u64le`/`be`) and
  **`Int.wrapping_add`/`wrapping_sub`/`wrapping_mul`** for hash finalizers
  and bit-mixing code that needs to overflow on purpose.
- **`fft.magnitude(re, im)`** and **`Range.any`/`Range.all`**
  (short-circuiting, matching `List`'s).
- **`worker.try_recv()`** — the non-blocking twin of `recv`, for a parent
  polling several workers without picking one to block on.
- **A new `std.lazy` module** (`Lazy[T]`: deferred, memoized computation),
  ergonomic `std.json` construction (`json.obj`/`arr`/`jstr`/`num`/…), and
  `strings.Builder` ergonomics (`is_empty`, `push_joined`).
- **`fable test --bless`** — rewrites a mismatched `//? expect:` line in
  place when the value changed but the print statements around it didn't,
  instead of making you retype it.

Everything observable is pinned: 309 golden spec tests, 134 executable book
snippets, and 71 demo golden tests, the whole suite green under
`FABLE_GC_STRESS=1`.

## The demo zoo

Every one of the seventeen [`demos/`](https://github.com/memmam/fable/tree/main/demos)
— a Lisp, a spreadsheet, a backtracking regex engine, checkers with an
alpha-beta engine, a from-scratch PNG encoder, a chiptune renderer, a
parallel Mandelbrot, and ten more — ships as a **self-contained binary**: no
`fable`, no source, one file you run. They are attached as
`fable-demozoo-v0.8.0-<target>.tar.gz` for five desktop targets:

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
