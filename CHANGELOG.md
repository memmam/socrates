# Changelog

Each release was shipped as one reviewed pull request; the book documents
them narratively (chapters 7–10 for v0.2 onward). Golden spec tests pin
every feature listed here.

## Unreleased (v0.7 — the infrastructure release, in progress)

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
