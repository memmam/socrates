# Fable — project memory

Fable is a statically-typed, garbage-collected programming language with
algebraic data types, exhaustive pattern matching, closures, and generics,
implemented from scratch in Rust with **zero dependencies** (default build).
This file is the working memory for the project: what Fable is *for*, the
invariants that must never break, the engineering principles that decide
close calls, how to verify a change, where the detailed records live, and a
terse release ledger to draw on when writing changelogs and release posts.

## What Fable is for

Fable is an **AI-native language**: its design mirrors the way current
frontier models reason, so an AI writes it fluently and uses it as a
recursive force multiplier — a substrate for building the tools that build
more tools. When AI-authorship fluency and human readability/ergonomics
pull in different directions, AI-fluency wins — this is not a language
optimized for a human encountering it cold. Naming that mirrors a
well-known API an AI already has deep fluency in (GLM's `vec3`/`perspective`/
`look_at`, GLSL/TSL shapes, POSIX/Win32 call shapes) beats naming chosen for
human intuition every time the two conflict. The intended trajectory, in
rough order:

- **Agent tooling** — MCP servers, client/harness code, glue — written in a
  language an AI can produce correctly on the first pass and golden-test
  instantly.
- **Hardware testing → an HDL pipeline.** Bit-exact integer semantics,
  `Bytes`, and bitwise intrinsics are load-bearing here; the near-term
  target is the user's 9-bit toy ISA (an AI-coprocessor design) — an
  assembler, emulator, and conformance battery in Fable, then codegen.
- **Transpilation *into* Fable** from Python and JavaScript and their
  frameworks (numpy/scipy already shimmed as `pyl`; Three.js-class and AIML
  frameworks next). The `ports/` programme (`jsl`, `pyl`) is the seed: run
  upstream unmodified against a Fable-backed shim, cross-validate to
  numeric/pixel equality.
- **Transpilation *from* Fable to raw Rust**, reaching for `unsafe`,
  pre-existing binary blobs, or external dependencies **only where
  necessary** — the same zero-dep-by-default discipline the interpreter
  holds itself to.

Keep this arc in mind when weighing features: the ones that serve
AI-authorship, bit-exact systems work, parallelism, and transpilation earn
their place fastest.

## Engineering principles (how to decide close calls)

- **Fastest, most parallelizable idiom wins.** When several ways express the
  same thing, prefer the one that is fastest *and* cleanest to parallelize,
  without losing capability. If picking it would drop functionality, add a
  new function or a thin wrapper — whichever is more performant — so nothing
  is lost. Err toward simplicity and performance.
- **Parallel compute is a first-class citizen**, not an afterthought:
  workers (OS-thread isolates), and the feature-gated GPU path, are core,
  and new hot paths should be designed to fan out cleanly.
- **The most performant version of an idiom becomes the primitive; older
  spellings become minimal wrappers over it** (the efficiency-pass rule —
  hand-rolled popcount/ushr/hex became one-line wrappers over natives).
  Every such change stays byte-identical in observable behavior.
- **This applies to whole backend implementations, not just algorithmic
  idioms.** When a newer backend for the same capability is more optimal on
  a platform (Metal over OpenGL on macOS; eventually Vulkan/DirectX over
  GL elsewhere), the newer one supersedes rather than living alongside the
  old one indefinitely — the older path is wrapped, thinned, or dropped as
  the better one lands. The Fable-facing API is the stable surface across
  that swap (a windowing layer shared across backends, e.g.); the backend
  underneath it is free to change as long as the observable output stays
  testably correct (golden tests, pixel/numeric cross-checks — whatever
  the feature's own verification story is). Don't build an interim backend
  you already know will be thrown away once the better one is in scope.

## Invariants (do not break these)

- **Zero dependencies in the default build.** `cargo tree -e normal` is one
  line. The only optional deps are `wgpu`/`pollster`, gated behind the `gpu`
  cargo feature (`--features gpu`); the default build never pulls them.
- **`docs/SPEC.md` is the source of truth.** The implementation, the golden
  tests, and the book must all agree with it; a deviation is a bug. Any
  language change updates the spec in the same PR.
- **Everything observable is golden-tested and byte-identical.** Every
  demo's full stdout is pinned (`demos/`), every ```fable block in `book/`
  executes in CI, and the spec suite (`tests/spec/`, 309 tests) runs through
  the same `fable test` path users get. A refactor that changes any pinned
  output is wrong unless the output change is the point.
- **GC-stress must stay green.** `FABLE_GC_STRESS=1` collects before every
  allocation; the whole suite (unit, spec, demos) passes under it. New
  natives that allocate must root correctly (see `temp_roots` in `vm.rs`).
- **Seeded randomness is stable only within a release**, never across.
  Corpora that must outlive a release use a hand-rolled PRNG, not `math.seed`.

## The gauntlet (run before shipping any interpreter change)

```sh
cargo test                                    # unit + golden spec suite
FABLE_GC_STRESS=1 cargo test --test spec_runner
cargo clippy --all-targets -- -D warnings
cargo build --release
./target/release/fable test tests/spec        # 309
./target/release/fable test demos             # 71, also with FABLE_GC_STRESS=1
FABLE_PATH=ports ./target/release/fable test ports/pyl/spec.fable   # + ports/icaa/spec.fable
./target/release/fable build demos/csvql -o /tmp/csvql && (cd /tmp && ./csvql)  # `fable build` smoke
bench/run.sh 3                                # perf A/B vs a pre-change binary
```

Performance claims are only real if `bench/run.sh` reproduces them
interleaved against a pre-change binary; see `bench/RESULTS.md` for method
and the standing numbers.

## Where the detailed memory lives

- `CHANGELOG.md` — per-release feature list (each release shipped as one PR).
- `docs/SPEC.md` — the normative language reference (`(vN)` tags mark when a
  feature landed).
- `docs/ARCHITECTURE.md` — implementation internals, module by module.
- `docs/RELEASING-macOS.md` — one-time setup to turn on Developer ID signing +
  notarization for the macOS demo-zoo binaries (the six repo secrets).
- `bench/RESULTS.md` — benchmark methodology + the efficiency-pass deltas.
- `demos/NOTES.md` — the field-test triage ledgers: every papercut demo
  authors hit, and whether it was fixed / documented / declined. The raw
  material for "what usage pulled in" in a release post.
- `demos/STYLE.md` — best-practice house rules distilled from the demo
  rounds (golden discipline, determinism, bitwise, workers, std collections).
- `ports/*/CONTRACT.md`, `ports/*/README.md` — the porting programme
  (SkyeShark's ICAA in `jsl`; claudewave in `pyl`), including how each port
  is cross-validated against its upstream.
- `book/` — the language book (a teaching resource, **not** a project diary;
  process/history belongs here in `CLAUDE.md` and the files above, never in
  the book).

## Release ledger (source material for release posts)

- **v0.1** — the language: lexer, parser, unification inference with
  generics, Maranget exhaustiveness, bytecode compiler, stack VM, mark-sweep
  GC, REPL, formatter, disassembler, golden-test harness, spec, book.
- **v0.2** — impl blocks (methods on user types), the `?` operator,
  multi-file modules (`import`, diamond dedup, cycle detection), tail-call
  optimization.
- **v0.3** — `pub` visibility (private-by-default modules), operator methods
  (`add`/`sub`/…), `FABLE_PATH` search path, `fs`/`os` namespaces.
- **v0.4** — `fable test` command, the embedded standard library
  (json/flags/path/strings/iter, written in Fable), `fable lsp`, catchable
  panics (`try`).
- **v0.5** — REPL imports, LSP completion, the book became a CI test suite.
- **v0.6 — the field-test release.** Ten demo programs written by ten
  independent authors under orders to report every papercut; ten hit the
  same dozen walls, and the release is those walls removed. One genuine RNG
  bug their tests caught: `math.seed` set state to `seed | 1`, so adjacent
  seeds `2k`/`2k+1` produced identical streams; fixed with SplitMix64
  scrambling. Triage in `demos/NOTES.md`.
- **v0.7 — the infrastructure release.** `Bytes` (packed buffers, binary
  I/O, wire formats); native `fft` namespace (radix-2 + Bluestein, numpy
  conventions, CI-cross-checked at 1e-9); OS-thread `worker` isolates with
  string channels; Int bitwise operators (`& | ^ << >>`) plus intrinsics
  (`count_ones`/`ushr`/`rotate_*`/`to_hex`) and Bytes readers/BE pushers;
  feature-gated `gpu` compute (wgpu, first optional dep, default build stays
  zero-dep); a std collections layer (`set`/`deque`/`lists`, `strings.Builder`);
  a line-width-aware formatter. Two ports validated to numeric/pixel
  equality (ICAA 18/18 pixel-exact; claudewave 32/32 battery, 29 bit-exact).
  Then a measured efficiency pass (see `bench/RESULTS.md`): checkers −15%,
  lisp −20%, string building −55%, map ops −37%, GC-stress suite −67% on the
  heaviest demo — all with byte-identical golden output. Finally, distribution:
  `fable build` staples a program (modules, data files, worker `.fable`s) onto
  the interpreter as an appended, dependency-free payload the binary reads from
  its own tail at startup — a self-contained executable whose output is
  byte-identical to the source run. Target-independent stapling (`--launcher`)
  lets one host cross-build the whole **demo zoo**: all seventeen demos for
  `x86_64`/`aarch64` Linux + Windows and Apple-Silicon macOS, shipped in the
  release. macOS can't append (a payload past the Mach-O `__LINKEDIT` fails
  `codesign` and arm64 won't run unsigned), so there the payload is linked in
  as a `__DATA,__fablezoo` section (`ld -sectcreate`; `fable build
  --payload-only` emits it; `read_self` parses the running image to read it
  back) and ad-hoc signed. Developer ID signing + notarization are wired in
  `release.yml`, dormant until the `MACOS_CERT_P12_BASE64` etc. secrets exist.
- **v0.8 — the demo round's feature queue.** The v0.7 demo round left a
  ranked, deduplicated feature-request queue (`demos/NOTES.md`); this
  release works through it directly rather than via a fresh round.
  `if let`/`while let` (pure parser sugar, desugared fully to `match`/`while`
  at parse time — the checker and compiler need no special cases); bitwise
  compound assignment (`&= |= ^= <<= >>=`); hex/binary literals now express
  the full 64-bit pattern (bit 63 included) plus `String.parse_hex()`;
  `Bytes` 64-bit accessors (`push`/`read_u64le`/`be`); `Int.wrapping_add`/
  `sub`/`mul`; `fft.magnitude`; `Range.any`/`all`; non-blocking
  `worker.try_recv()`; `strings.Builder.is_empty`/`push_joined`;
  `lists.min_by_key`/`max_by_key`; a new `std.lazy` module (`Lazy[T]`,
  deferred/memoized computation); ergonomic `std.json` construction
  (`obj`/`arr`/`jstr`/`num`/`int`/`bool`/`null`); and `fable test --bless`,
  which rewrites mismatched `//? expect:` lines in place when the
  actual/expected count already agrees. One item (a counting-map helper)
  declined — one demo, one-line workaround, `std` grows reluctantly. Four
  items in the original queue turned out to already be shipped in v0.7's
  own efficiency pass; `demos/NOTES.md` now says so.

## Workflow conventions

- Merge-on-green: CI is trusted; feature PRs are real (non-draft) with
  auto-merge armed — drafts are reserved for *releases*. Feature work
  happens on a dedicated branch off `main`.
- Commit messages state what changed and (for perf) the measured delta.
- The spec, the book's executable snippets, and the demos' pinned output are
  the three tripwires — if a change is wrong, one of them goes red.
