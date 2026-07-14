The first packaged release of Fable: a statically-typed, garbage-collected
programming language — ADTs, exhaustive pattern matching, closures,
generics, modules, a test runner, a language server, a REPL, and an
embedded standard library — implemented from scratch in ~18,000 lines of
Rust with **zero dependencies**.

v0.6 is the *field-test release*: ten demo programs (`demos/` — a Lisp, a
spreadsheet, a regex engine, checkers, wave-function collapse, and five
more) were written against v0.5 with orders to report every papercut, and
this release is their reports answered — including one genuine RNG bug
their golden tests caught. The full triage is
[`demos/NOTES.md`](https://github.com/memmam/fable/blob/main/demos/NOTES.md);
the narrative is
[book chapter 10](https://github.com/memmam/fable/blob/main/book/10-field-test.md).

Highlights of v0.6 over v0.5 (full history in
[`CHANGELOG.md`](https://github.com/memmam/fable/blob/main/CHANGELOG.md)):

- `for (i, x) in xs.enumerate()` — for-loop heads take irrefutable patterns
- Bare `return` / `break` / `continue` match arms; `while true` divergence
- `_` lambda parameters; `os.exit` typechecks anywhere, like `panic`
- Strings: `trim_start`, `trim_end`, `code_at`, `index_of_from`, free `char()`
- `Float.to_fixed(n)`; `math.rand_int` / `log10` / `fmod`
- **Fixed:** `math.seed` collapsed adjacent seeds (42 and 43 were identical)
- **Fixed:** `fable test` no longer parses `//?` inside strings or prose
- `FABLE_MAX_DEPTH` env var for the call-depth cap; sharper diagnostics

Everything is pinned: 262 golden spec tests, 112 executable book snippets,
39 demo golden tests, the whole suite green under `FABLE_GC_STRESS=1`.

**Getting started:** unpack the attached Linux binary (or
`cargo build --release` from source on any platform Rust supports), then:

```sh
fable examples/mandelbrot.fable
fable demos/checkers/main.fable      # watch it play itself
fable test tests/spec demos          # run the whole golden suite
fable repl
```

Licensed under the Apache License 2.0 — see `LICENSE` and `NOTICE`.
