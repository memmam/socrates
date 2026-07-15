# Changelog

Each release was shipped as one reviewed pull request. Golden spec tests pin
every feature listed here; `docs/SPEC.md` marks each with the version that
introduced it, and `CLAUDE.md` keeps the release ledger.

## v0.7.0 — the infrastructure release

- `fable build <dir|file>` — pack a program into one self-contained
  executable. Every file the program touches (modules, data files, the
  `.fable` files it hands `worker.spawn`) is stapled onto a copy of the
  interpreter as a dependency-free appended payload; the binary reads its
  own 16-byte trailer at startup, unpacks into a scratch directory, and
  runs — so its output is byte-identical to `fable <path>` from source.
  Stapling is target-independent (`--launcher` supplies cross-compiled
  interpreter bytes), so one host assembles binaries for every target. The
  release ships the whole **demo zoo**: all seventeen demos cross-built for
  `x86_64`/`aarch64` Linux and Windows and Apple-Silicon macOS, as
  `fable-demozoo-<version>-<target>.tar.gz`. On macOS — where a payload
  appended past the Mach-O `__LINKEDIT` can't be code-signed — the payload is
  linked in as a `__DATA,__fablezoo` section instead (`ld -sectcreate`; read
  back by parsing the running image) and ad-hoc signed; Developer ID signing +
  notarization are wired to switch on once the signing secrets are configured.
- The efficiency pass: a measured, benchmark-gated optimization sweep
  (`bench/` is the yardstick; every change was interleaved-A/B'd, and
  negative results are recorded in the audit trail). Interpreter: frame-
  hot state hoisted into dispatch-loop locals, write-in-place binop and
  native-call stack traffic, allocation-free `for` over Int ranges,
  scalar fast paths for structural hashing, interned single-char ASCII
  strings, allocation-free GC mark phase with out-of-line mark bits,
  FMap single-entry buckets without SipHash. Natives: borrow-based
  string/list methods, pre-sized joins and HOF outputs. std: Builder
  re-backed by Bytes, json over UTF-8 bytes, deque/lists/set hot paths
  simplified. Net (interleaved vs pre-pass): checkers −15%, lisp −20%,
  string building −55%, map ops −37%, dispatch micros −14..19%,
  GC-stress suite time −67% on the heaviest demo. All 294 spec and 71
  demo goldens byte-identical throughout.

- Fast-idiom natives (the efficiency pass, batch 1): every bit-heavy
  demo in the v0.7 round hand-rolled the same primitives, so they are
  now intrinsics. `Int` grew `count_ones` / `leading_zeros` /
  `trailing_zeros` (the 0 case is 64 for both zero-counts, matching
  Rust), `ushr` (logical right shift, `>>`'s exact panic contract),
  `rotate_left` / `rotate_right` (count mod 64; never panic), and
  `to_hex` (lowercase minimal hex of the two's-complement pattern).
  `Bytes` grew bulk appends `push_bytes` (snapshot semantics —
  self-append works) / `push_str`, big-endian pushers `push_u16be` /
  `push_u32be` (same range checks as the LE trio), and multi-byte
  readers `read_u16le` / `read_i16le` / `read_u32le` / `read_u16be` /
  `read_u32be` (OOB panics match `get`). **The wrappers rule:** the
  demos' hand-rolled versions did not disappear — each became a
  minimal wrapper over the native with the same name and byte-identical
  observable behavior (`reversi/bits.fable` remains the documented
  reference; the hand-rolled bodies live in git history).
- The v0.7 demo round: six new demos (`synthwave`, `png`, `bloom`,
  `spectra`, `swarm`, `reversi`) built on the new infrastructure, all
  eleven existing demos modernized to it, seventeen writers plus
  seventeen adversarial verifiers. Best practices distilled into
  `demos/STYLE.md`; the papercut triage is `demos/NOTES.md` § "The
  v0.7 round".
- **Fixed (found by the round):** method calls and field access on
  module-qualified `pub let` members (`m.answer.to_float()`) no longer
  misresolve as enum paths; `worker.spawn` resolves relative files
  against the true entry script's directory even when the entry has
  imports; `fable fmt` formats every file argument (it silently took
  only the first); the formatter keeps interior comments of bracketed
  literals in place (they now pin the element-per-line layout — the
  official escape hatch for 2-D data tables); fitting `if/else-if`
  chains stay on one line and over-width ones break all branches
  consistently.

- `fable fmt` is now line-width-aware (v0.6 review note): constructs
  that fit in 100 columns keep their one-line layout; longer ones break
  the way an author would — call arguments one per line with a trailing
  comma, method chains before each `.` after the first, binary
  expressions before each operator, literals element-per-line, lambda
  bodies to a block — composing outermost-first. `--width N` overrides
  the limit; tokens are never split; comments and `//?` directives are
  preserved; formatting stays idempotent and behavior-preserving.
- `Bytes`: a packed byte-buffer primitive with checked accessors,
  little-endian multi-byte pushers (wire formats without bitwise
  operators), `slice`/`concat`/`to_list`, UTF-8 bridging to `String`,
  structural equality, and map-key support.
- `fs.read_bytes` / `fs.write_bytes` — binary file I/O, surfaced as a
  hard gap by the claudewave port (audio output needed WAV).
- `fft` namespace: native `fft.fft` / `fft.ifft` / `fft.rfft` over
  split-complex signals, any length ≥ 1 in O(n log n) (radix-2 for
  powers of two, Bluestein otherwise); numpy conventions,
  cross-checked against numpy in CI at 1e-9.
- Bitwise operators on Int: `&` `|` `^` `<<` `>>` (Rust's relative
  precedence; `>>` arithmetic; shift counts outside 0..=63 panic).
  The v0.6 "no bitwise operators" diagnostics retired.
- Workers: `worker.spawn(file, args)` runs a Fable program as an
  **isolate** — its own VM, heap, and GC on its own OS thread — joined
  to the parent by string channels (`send`/`recv`/`join` on the
  `Worker` handle; `worker.send`/`worker.recv`/`worker.is_worker`
  inside). Compile errors surface synchronously from `spawn`; a
  worker's panic is isolated and comes back as `Err` from `join`.
- `ports/`: the porting programme — `jsl` (JS/TSL layer, ICAA port,
  cross-validated to pixel equality) and `pyl` (Python/numpy layer,
  claudewave DSP core, in progress).
- **`gpu` namespace (experimental, feature-gated)**: `gpu.available()`,
  `gpu.adapter_info()`, and `gpu.run(wgsl, input, out_len, wx, wy, wz) ->
  Result[Bytes, String]` dispatch WGSL compute shaders over `Bytes` I/O
  (SPEC §7.2 documents the shader ABI). Implemented with wgpu behind the
  `gpu` cargo feature — the project's first-ever dependency, deliberately
  quarantined so the **default build stays zero-dependency** (CI asserts
  it). Without the feature the namespace still typechecks and degrades
  gracefully (`available()` is `false`, `run` returns `Err`). Demo:
  `docs/assets/gpu_double.fable`.
- `std` grows a collections layer: `std.set` (`Set[T]` over structural
  map keys; `insert`/`remove` report change, `union`/`intersect`/
  `difference` preserve insertion order), `std.deque` (two-stack
  `Deque[T]`, amortized O(1) at both ends), `std.lists` (`fill`,
  `sum`/`sum_float`, `min`/`max`/`min_float`/`max_float`,
  `min_by`/`max_by` — `sort_by` comparators, first winner on ties),
  and `strings.Builder` (`builder()`, `push`/`push_char`/`len`/
  `build`/`clear`) for O(n) string accumulation where a `+=` loop
  is O(n²).

## v0.6.0 — the field-test release

Ten demo programs (`demos/`) were written against v0.5 with orders to
report every papercut; ten independent authors hit the same dozen walls.
This release is those walls removed. Full triage: `demos/NOTES.md`.

- **Fixed:** `math.seed` collapsed adjacent seeds (42 and 43 produced
  identical streams); now SplitMix64-scrambled. Seeded streams are not
  stable across releases.
- **Fixed:** `fable test` treated `//?` anywhere in a line — including in
  strings and prose comments — as a directive; now it must begin the
  line's comment (full lexical model: strings, interpolation holes,
  nested block comments).
- `for` heads take irrefutable patterns: `for (i, x) in xs.enumerate()`.
- Bare `return`/`break`/`continue` as match-arm bodies; diverging arms
  unify with any type.
- A trailing `while true` with no `break` diverges; `os.exit` typechecks
  in any value position, like `panic`.
- `_` as a lambda parameter.
- Strings: `trim_start`, `trim_end`, `code_at`, `index_of_from`; free
  `char(code)`. Floats: `to_fixed(n)`. Math: `rand_int` (uniform via
  rejection sampling), `log10`, `fmod`.
- `FABLE_MAX_DEPTH` env var raises the 4,096-frame call-depth cap.
- Golden comparison ignores trailing whitespace; `fable test` rejects
  unknown flags.
- Targeted diagnostics: `{}` vs `{:}`, assignment as a match-arm body
  (including `+=`), `<<`/`>>`.
- Spec: struct-literal field shorthand documented (long-standing
  behavior), sorts documented stable, `math.log` documented natural.
- The v0.6 diff itself was adversarially reviewed before shipping; all
  confirmed findings fixed in-release (`demos/NOTES.md` § "The review
  round").

## v0.5.0 — closing the loop

- REPL imports: modules (including `std.*`) load, persist, and roll back
  cleanly across inputs.
- LSP completion: methods, fields, tuple indices, module members,
  namespaces, and top-level names — answered from the last good analysis,
  so it works mid-edit.
- The book became a test suite: every ```fable block in `book/` executes
  in CI, with fence tags for deliberate errors/panics.

## v0.4.0 — the toolchain release

- `fable test`: any `.fable` file with `//? expect/error/panic`
  directives is a golden test; the spec suite runs through the same code.
- Embedded standard library: `std.json`, `std.flags`, `std.path`,
  `std.strings`, `std.iter` (lazy iterators) — written in Fable, compiled
  into the binary.
- `fable lsp`: diagnostics, hover, go-to-definition over stdio; JSON-RPC
  hand-rolled.
- `try(f)`: catchable panics with full VM-stack restoration.
- GC pacing tuned (closure-churn benchmark 161ms → 38ms).

## v0.3.0 — the glue release

- `pub` visibility: module items private by default.
- Operator methods: `+ - * / %` and unary `-` dispatch to user methods;
  `==` stays structural.
- `FABLE_PATH` module search path.
- `fs.*` and `os.*` namespaces, Result-based and `?`-friendly.

## v0.2.0 — everything v0.1 declared out of scope

- `impl` blocks: methods on user structs and enums, generic impls.
- The `?` operator for `Option` and `Result`.
- Multi-file modules: `import a.b;` with diamond dedup and cycle
  detection.
- Tail-call optimization: frame reuse for calls in tail position.

## v0.1.0 — the language

Lexer, parser, unification-based inference with generics, Maranget
exhaustiveness checking, bytecode compiler, stack VM, mark-sweep GC,
REPL, formatter, disassembler, golden-test harness, spec, book, and
examples — zero dependencies.
