# 7. Methods, `?`, Modules, and Tail Calls

Everything v0.1 declared out of scope, v0.2 ships: methods on your own types
(`impl` blocks), the `?` operator, multi-file programs (`import`), and
tail-call optimization. This chapter tours all four. As everywhere in this
book, every snippet was run against the real interpreter.

## 7.1 impl blocks: methods on your types

v0.1 Fable had methods — `xs.map(f)`, `s.trim()` — but only on builtin types.
Your own structs and enums made do with free functions and the argument order
you could remember. v0.2 adds `impl`:

```fable
struct Point { x: Float, y: Float }

impl Point {
    fn len(self) -> Float {
        (self.x * self.x + self.y * self.y).sqrt()
    }

    fn scaled(self, k: Float) -> Point {
        Point { x: self.x * k, y: self.y * k }
    }
}

let p = Point { x: 3.0, y: 4.0 };
println(p.len());               // 5.0
println(p.scaled(2.0).len());   // 10.0
```

The first parameter of every method is a bare `self` — no type annotation,
because it always has the impl type. Everything else about a method is an
ordinary function: parameters are typed, the return type defaults to `Unit`,
recursion and mutual calls work, and methods are hoisted, so a function above
the impl block can call methods declared below it.

Methods shine on enums, where they pair with `match`:

```fable
enum Tree {
    Leaf(Int),
    Node(Tree, Tree),
}

impl Tree {
    fn sum(self) -> Int {
        match self {
            Tree.Leaf(v) -> v,
            Tree.Node(l, r) -> l.sum() + r.sum(),
        }
    }
}

let t = Tree.Node(Tree.Node(Tree.Leaf(1), Tree.Leaf(2)), Tree.Leaf(4));
println(t.sum());   // 7
```

Generic types re-bind their type parameters in the impl header, by position;
methods may add their own generics after the impl's:

```fable
struct Pair[A, B] { first: A, second: B }

impl Pair[A, B] {
    fn swap(self) -> Pair[B, A] {
        Pair { first: self.second, second: self.first }
    }

    fn replace_first[C](self, c: C) -> Pair[C, B] {
        Pair { first: c, second: self.second }
    }
}

let p = Pair { first: 1, second: "one" };
println(p.swap().first);            // one
println(p.replace_first(true).first);   // true
```

Rules worth knowing:

- Multiple impl blocks per type are fine (organize as you like); a method
  name may only be defined once per type across all of them (E0333).
- Impl targets must be your own structs or enums. `impl Int` or
  `impl Option` is an error (E0331) — builtins keep their curated method
  sets.
- Under the hood a method is just a function whose first argument is the
  receiver; `p.scaled(2.0)` compiles to the same `call_fn` as
  `scaled(p, 2.0)` would. `fable dis` will show you.

## 7.2 The `?` operator

The `Option`/`Result` combinators (`map`, `and_then`, `unwrap_or`) cover a
lot, but deep chains of "give me the value or bail" used to mean nested
`match`es. The `?` operator is that pattern, spelled in one character:

```fable
fn parse_sum(a: String, b: String) -> Option[Int] {
    Some(a.parse_int()? + b.parse_int()?)
}

println(parse_sum("2", "40"));    // Some(42)
println(parse_sum("2", "forty")); // None
```

`expr?` unwraps `Some`/`Ok` and keeps going, or returns the `None`/`Err`
from the enclosing function on the spot. The rules:

- On `Option[T]`: yields the `T`; the enclosing function must return an
  `Option` (any payload type).
- On `Result[T, E]`: yields the `T`; the enclosing function must return a
  `Result` with the **same error type** `E` (any success type) — the `Err`
  travels through unchanged.
- Anywhere else — a non-`Option`/`Result` operand (E0330), a function whose
  return type doesn't match (E0329), or top-level code with no function to
  return from (E0328) — it's a compile error, not a surprise at runtime.

It binds as a postfix operator, tighter than unary minus, so chains read
left to right: `config.lookup("port")?.parse_int()?`. Inside a lambda, `?`
returns from the lambda.

The JSON parser in `examples/json.fable` is the before/after story: every

```fable skip
match parse_value(p) {
    Ok(v) -> items.push(v),
    Err(e) -> { return Err(e); }
}
```

became

```fable skip
items.push(parse_value(p)?);
```

## 7.3 Modules: programs across files

A Fable program can now span files. `import` names a sibling file (or a
subdirectory path), and everything in it is reachable through the module
name:

```fable
// geo.fable
pub struct Point { x: Float, y: Float }

impl Point {
    pub fn dist(self, other: Point) -> Float {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

pub fn origin() -> Point {
    Point { x: 0.0, y: 0.0 }
}
```

```fable
// main.fable — run this one
import geo;

let p: geo.Point = geo.Point { x: 3.0, y: 4.0 };
println(p.dist(geo.origin()));   // 5.0
```

`import a.b;` loads `a/b.fable` relative to the importing file and binds it
as `b`; `import a.b as m;` picks the name. Qualification works everywhere a
name can appear: function calls (`geo.origin()`), globals (`geo.gravity`),
types (`geo.Point`), variant constructors (`geo.Shape.Circle(1.0)`), struct
literals (`geo.Point { .. }`), and patterns (`geo.Shape.Circle(r) -> ..`).
Two things need no qualification at all: methods — they travel with their
type, so `p.dist(..)` works on a `geo.Point` anywhere — and variant patterns
in a `match` whose scrutinee type is known, exactly like `Some`/`None`.

The semantics are deliberately simple:

- A module loads **once** per program, no matter how many files import it
  (diamonds share state), and its top-level code runs once, before any
  importer's. The root file runs last.
- Circular imports are a compile error with the cycle spelled out (E0338).
- Module items are private unless marked `pub` (added in v0.3 — chapter 8).
  `pub` module globals are readable from outside (`geo.gravity`) but only
  the owning module can assign them.
- Errors and panics point into the right file — stack traces span modules.

`examples/orbit/` is a working two-file program: `vec.fable` is a vector
module with impl methods, and `main.fable` integrates a three-body system
with it.

## 7.4 Tail calls: recursion without a budget

v0.1 capped the call stack at 4,096 frames — fine for tree walks, fatal for
a recursive main loop. v0.2 compiles calls in **tail position** to a frame
reuse instead of a push, so this runs in constant stack space:

```fable
fn count_down(n: Int, acc: Int) -> Int {
    if n == 0 { acc } else { count_down(n - 1, acc + 1) }
}
println(count_down(1000000, 0));   // 1000000 — one frame, a million calls
```

Tail position is where a call's result immediately becomes the function's
result: the operand of `return`, the last expression of a function or lambda
body, and the result position of an `if`/`match`/block sitting in tail
position. Mutual recursion qualifies (`is_even`/`is_odd` bounce in one
frame), as do calls through function values.

What does *not* qualify is anything with work left to do after the call:

```fable
fn sum_to(n: Int) -> Int {
    if n == 0 { 0 } else { n + sum_to(n - 1) }   // the `+` runs after — not a tail call
}
```

Deep enough, that still panics with `stack overflow` — the honest outcome,
since each frame really is holding a pending `+`. Rewrite with an
accumulator and the optimizer takes it from there.

The bytecode makes the difference visible: `fable dis` shows `tail_callfn`
where a frame is reused and plain `call_fn` where one is pushed.

## 7.5 All together

The four features compose. A module exports a type; the type carries
methods; the methods return `Result`s that `?` threads through; and the
driver loop at the bottom is tail-recursive. That's `examples/orbit/` and
the rewritten `examples/raytracer.fable` and `examples/json.fable` — the
same programs as v0.1, now reading the way they always wanted to.

---

Previous: [Under the Hood](06-under-the-hood.md) ·
Next: [The Glue Chapter](08-glue.md) ·
[Back to the index](README.md)
