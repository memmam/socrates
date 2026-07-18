# Error Handling

Socrates has no exceptions. A function that can fail says so in its return
type, using the two prelude enums from the last chapter: `Option[T]` for
"a value or nothing", and `Result[T, E]` for "a value or an explanation".
This chapter is the toolkit for working with them — combinators for the
common shapes, the `?` operator for propagation, `try` for the rare case
where you need to catch a panic, and a clear account of what panics are and
when they happen.

## Combinators: handling failure without a full `match`

A `match` is always available, but for the common moves there are methods.
`unwrap_or` supplies a default, `map` transforms the success case and leaves
the failure untouched, and `and_then` chains another fallible step:

```soc
let n = "42".parse_int();      // Option[Int]

println(n.map(|v| v * 2));                 // transform if present
println(n.unwrap_or(0));                   // value, or a default
println("x".parse_int().unwrap_or(-1));    // the miss takes the default

let r: Result[Int, String] = Ok(10);
println(r.map(|v| v + 1));                              // Ok, transformed
println(r.and_then(|v| if v > 5 { Ok(v) } else { Err("too small") }));
```

```text
Some(84)
42
-1
Ok(11)
Ok(10)
```

`unwrap()` is the blunt instrument: it returns the inner value or *panics*
if there isn't one. It is right for cases you have already proven cannot
fail, and for throwaway code; in anything that must stay running, prefer
`unwrap_or`, a `match`, or the `?` operator below. `Option` also has
`is_some`/`is_none`, and `Result` has `is_ok`/`is_err`, `unwrap_err`, and
`map_err` for transforming the error side.

## The `?` operator

Deep chains of "give me the value or bail" are the reason `?` exists. It is
that pattern spelled in one character:

```soc
fn parse_sum(a: String, b: String) -> Option[Int] {
    Some(a.parse_int()? + b.parse_int()?)
}

println(parse_sum("2", "40"));      // Some(42)
println(parse_sum("2", "forty"));   // None — the second parse bailed
```

```text
Some(42)
None
```

`expr?` unwraps a `Some`/`Ok` and keeps going, or returns the `None`/`Err`
from the enclosing function immediately. The rules are checked at compile
time, so a misuse is an error, never a surprise at runtime:

- On `Option[T]` it yields the `T`; the enclosing function must return an
  `Option` (of any payload type).
- On `Result[T, E]` it yields the `T`; the enclosing function must return a
  `Result` with the **same error type** `E`, and the `Err` travels through
  unchanged.
- Used on a non-`Option`/`Result` value, in a function whose return type
  doesn't match, or at the top level where there is nothing to return from,
  it is a compile error.

`?` is a postfix operator that binds tighter than the arithmetic operators,
so chains read left to right: `config.lookup("port")?.parse_int()?`. Inside
a lambda, `?` returns from the lambda.

## `try`: catching a panic

Some failures are not modeled as values — an index out of bounds, an
`unwrap` on `None`, integer overflow. These *panic*: they abort the program.
That is usually what you want, but not when you are fifty files into a batch
job and one is malformed. `try(f)` runs a function and turns a panic into a
`Result`:

```soc
let results = ["10", "0", "x"].map(|s| try(|| 100 / s.parse_int().unwrap()));
println(results);
```

```text
[Ok(10), Err("division by zero"), Err("called `unwrap()` on `None`")]
```

The VM restores its stack completely — even a caught stack overflow leaves a
working machine. Two honest caveats: side effects that happened before the
panic persist (`try` is a recovery boundary, not a transaction), and
`os.exit` still ends the process. It composes with `?`, so "run this risky
thing, propagate the failure" is one line. Reach for `try` at the boundary
of untrusted input or a plugin; do not use it to paper over a bug you could
prevent with a `match`.

## What panics, and what you see

A panic is a runtime error that unwinds the stack and aborts with exit code
70, a message, and a trace. The built-in operations that panic all do so on
conditions a program can usually check for first:

- indexing a list, string, or `Bytes` out of bounds, or a map with a
  missing key via `[]` (use `get`, which returns an `Option`);
- `unwrap()` / `unwrap_err()` on the wrong variant;
- integer division or modulo by zero, and integer overflow;
- an explicit `panic("message")`, or `assert` / `assert_eq` failing;
- a shift count outside `0..=63`.

```soc panics
let xs = [10, 20, 30];
println(xs[5]);
```

```text
panic: list index out of bounds: index 5, length 3
  at <script> (demo.soc:2:9)
```

The trace names each active call with its source location — across module
files, when a program spans several. The rule of thumb that keeps programs
honest: model the failures you expect with `Option`/`Result` and handle them
with combinators or `?`; let panics stand for the bugs and impossible states
you would rather crash on than continue past; and keep `try` for the
boundary where you genuinely must survive someone else's mistake.

---

Previous: [Collections, Strings, and Bytes](05-collections-and-strings.md) ·
Next: [Programs Across Files](07-modules.md) ·
[Back to the index](README.md)
