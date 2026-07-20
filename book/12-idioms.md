# Idioms and Style

The language has been laid out; this chapter is about using it well. None of
these are rules the compiler enforces — they are the habits that make Socrates
programs fast, clear, and easy to test. The through-line is a single
preference: when two spellings do the same job, choose the one that is
faster and parallelizes more cleanly, and reach for the primitive built for
the task rather than hand-rolling it.

## Prefer expressions to statements

Socrates is expression-oriented, so most "compute a value conditionally" code
needs no mutable variable. Let `if` and `match` produce the value directly:

```soc
let n = 0 - 5;
let sign = match n {
    m if m < 0 -> "negative",
    0 -> "zero",
    _ -> "positive",
};
println(sign);
```

```text
negative
```

A tail expression is the idiomatic function result; `return` is for early
exits, not the happy path. Reserve `let mut` and reassignment for genuine
accumulation — a running total in a loop — rather than for building up a
value an expression could produce outright.

## Model failure as values; keep panics for bugs

A function that can fail should say so in its type with `Option` or
`Result`, and callers should handle it with combinators or `?`. Save
`unwrap()` and the panicking operations for the cases you have proven cannot
happen — an invariant you would rather crash on than silently continue past.
Use `try` only at a real boundary (untrusted input, a subprocess), never as
a substitute for a `match` you could have written. Failure-as-values also
parallelizes: a worker returns a `Result` string, and the parent decides
what a failure means without a shared error channel.

## Reach for the primitive built for the job

Hand-rolled loops are where performance quietly leaks. The fast version
usually already exists:

- Build strings with `strings.Builder` or collect-then-`join`, never `+=` in
  a loop (that is quadratic).
- Test membership with a `set` whose `insert` returns whether the value was
  new, rather than scanning a list.
- Count set bits with `Int.count_ones()`, shift unsigned with `ushr`, format
  hex with `to_hex()` — the intrinsics are both faster and clearer than the
  bit-twiddling they replace, and they sidestep the traps of a
  signed-64-bit, panic-on-overflow `Int`. When a hash finalizer or bit
  mixer needs arithmetic to overflow on purpose, `wrapping_add`/`sub`/`mul`
  say so directly instead of a workaround that fights the checked operators.
- Pack binary with the `Bytes` pushers and readers, not manual arithmetic.

When you do need a shape the standard library lacks, wrap the fast primitive
in a thin function rather than reimplementing it slowly.

## Make parallel work parallel

Work that is embarrassingly parallel — rendering rows, crunching independent
jobs — belongs on workers, one program per core, each with its own heap. The
discipline that keeps such programs testable is **determinism by protocol**:
have workers communicate only through the channel, keep their output silent,
and let the parent decide the order it prints results, so the program's
output is fixed no matter how the threads interleave. Deal the whole batch
of jobs out first and collect afterward — channels buffer, so you get full
parallelism with no synchronization code of your own.

## Let the tools carry the weight

Golden-test everything with `//?` directives — pinning a program's complete
output is the cheapest possible regression net, and it is why this book and
the demo suite can be refactored fearlessly. Run `socrates fmt` before you
commit so diffs show intent, not whitespace. Point your editor at `socrates
lsp` and let the checker answer questions you would otherwise guess at.

## Design values

A few commitments shape the whole project, and they are worth knowing
because they explain the choices above. The interpreter has **zero
dependencies** by default — a feature you can only lose once, so it is
guarded. `docs/SPEC.md` is the **source of truth**: the implementation, the
tests, and this book all answer to it. `std` grows by pull, not by
roadmap: a module earns its place by a real program needing it, never
speculatively. And Socrates is built to be written
*by* automated tooling as much as by hand, which is why so much of it —
golden tests, deterministic output, a spec that rules — is about making a
program's behavior legible and checkable. Write Socrates so a machine could
verify it, and you will have written it well.

---

Previous: [The Toolchain](11-toolchain.md) ·
[Back to the index](README.md)
