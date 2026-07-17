# The Fable Language Specification

**Version 0.8** — This document is the normative reference for the Fable
programming language. Inline `(vN)` tags mark the release where a feature
landed. The implementation (`src/`), the golden test suite (`tests/spec/`),
and the book (`book/`) must all agree with this document.

Fable is a statically-typed, expression-oriented, garbage-collected programming
language with algebraic data types, pattern matching with exhaustiveness checking,
first-class functions with closures, and generics. Programs are single files executed
top-to-bottom as scripts; functions and types are hoisted so mutual recursion works.

---

## 1. Lexical structure

### 1.1 Comments

```fable
// line comment
/* block comment /* which nests */ still comment */
```

### 1.2 Identifiers and keywords

Identifiers match `[A-Za-z_][A-Za-z0-9_]*`. By convention, types and enum variants are
`UpperCamelCase`, values and functions are `snake_case` (not enforced).

Keywords (reserved, cannot be identifiers):

```
let mut pub fn struct enum impl import match if else while for in
return break continue true false
```

The words `as` (import aliases) and `self` (method receivers) are contextual:
they are ordinary identifiers everywhere else.

The words `and`, `or`, and `not` are ordinary identifiers (the operators are
spelled `&&`, `||`, `!`); using `and`/`or` in infix position gets a targeted
error (E0106) pointing at the symbolic spelling.

### 1.3 Literals

- **Int**: `0`, `42`, `1_000_000` (underscores may appear anywhere after the first
  digit), `0x2A`/`0X2A` (hex), `0b1010`/`0B1010` (binary). 64-bit signed.
  Decimal literals are range-checked as `i64`; hex/binary literals name the
  raw 64-bit bit pattern (parsed as `u64`, then reinterpreted as the
  two's-complement `Int` — v0.8), so every pattern `to_hex()` can produce,
  bit 63 included, is also writable as a literal (`0x8000000000000000` is
  `Int`'s minimum value; `0xffffffffffffffff` is `-1`). Out-of-range
  literals — decimal beyond `i64`, or hex/binary needing a 65th bit — are a
  compile error (E0105).
- **Float**: `3.14`, `0.5`, `1e9`, `2.5e-3` (`e` or `E`). A float literal must have a
  digit before the `.` (`.5` is invalid; `0.5` is required). `1.` is invalid; `1.0` is
  required. A float literal whose value overflows evaluates to `inf`.
- **Bool**: `true`, `false`.
- **String**: `"hello"`. Escapes: `\n \t \r \\ \" \0 \{ \}`, and `\u{1F600}` for
  Unicode scalar values. Strings are immutable UTF-8.
- **String interpolation**: `"x = {x}, sum = {a + b}"` — any expression inside `{ }`.
  The expression's value is converted with the same rules as `str()` (§ 8.1). A literal
  `{` is written `\{`. `}` outside an interpolation is a plain character.
- **Unit**: `()` — the unit value, of type `Unit`.

### 1.4 Operators and punctuation

```
+ - * / %  == != < <= > >=  && || !  & | ^ << >>  =  -> =>  . , : ; ( ) [ ] { }  .. ..=  _  ?
```

(`&` `|` `^` `<<` `>>` are the Int bitwise operators, v0.7; `|` also
delimits lambda parameters, disambiguated by position.)

---

## 2. Types

### 2.1 Primitive types

| Type     | Description                                     |
|----------|-------------------------------------------------|
| `Int`    | 64-bit signed integer. Arithmetic overflow **panics**. |
| `Float`  | IEEE-754 double.                                |
| `Bool`   | `true` / `false`.                               |
| `String` | Immutable UTF-8 string.                         |
| `Unit`   | The single value `()`.                          |
| `Bytes`  | Mutable packed byte buffer (v0.7): binary I/O and wire formats. Reference semantics; `==` is structural; not orderable. Displays as `<bytes N>`. |

There are **no implicit numeric conversions**. `1 + 2.0` is a type error; write
`1.to_float() + 2.0` or `1.0 + 2.0`.

### 2.2 Compound types

- `List[T]` — growable array. **Reference semantics** (aliases see mutation).
- `Map[K, V]` — hash map. Reference semantics. Keys are compared/hashed structurally;
  function-containing key types are rejected at compile time (E0312) when concrete,
  and panic at runtime when reached through a generic type parameter.
- `(T1, T2, ...)` — tuple, 2 or more elements. **Immutable, value semantics.**
  Accessed by pattern matching or `.0`, `.1`, ... index syntax.
- `fn(T1, T2) -> R` — function type.
- `Range` — produced by `a..b` (half-open) and `a..=b` (inclusive), `a`, `b: Int`.
  Iterable in `for`; also has methods (§ 8.7).

### 2.3 Structs

```fable
struct Point { x: Float, y: Float }
struct Pair[A, B] { first: A, second: B }

let p = Point { x: 1.0, y: 2.0 }
let q = Pair { first: 1, second: "one" }   // Pair[Int, String] inferred
p.x                    // field access
p.x = 3.0              // field assignment — struct instances are mutable heap objects
```

Structs are **nominal** and have **reference semantics**: `let a = p; a.x = 9.0`
changes `p.x` too. Structs must be constructed with **all** fields present, in any
order. `Point { x: 1.0 }` (missing field) and unknown fields are compile errors.

**Field shorthand** (v0.6, documenting long-standing behavior): a field whose
value is a variable of the same name may be written once — `Point { x, y }`
means `Point { x: x, y: y }`, and shorthand and explicit fields mix freely
(`Point { x, y: 2.0 }`). `fable fmt` canonicalizes `x: x` to the shorthand.

### 2.4 Enums (algebraic data types)

```fable
enum Shape {
    Circle(Float),
    Rect(Float, Float),
    Empty,
}
enum Option[T] { Some(T), None }        // built into the prelude
enum Result[T, E] { Ok(T), Err(E) }     // built into the prelude
```

Variants are referenced as `Shape.Circle(1.0)`, `Shape.Empty`. The prelude variants
`Some`, `None`, `Ok`, `Err` may be used **unqualified**; `Option.Some(x)` also works.
Enum values are immutable. Nullary variants are written *without* parentheses:
`Shape.Empty`, not `Shape.Empty()`.

### 2.5 Generics

Functions and types take explicit type parameters in square brackets:

```fable
fn identity[T](x: T) -> T { x }
fn map[T, U](xs: List[T], f: fn(T) -> U) -> List[U] { ... }
```

Type arguments are always **inferred at call sites** (`identity(5)`); there is no
turbofish syntax. Generics are type-erased at runtime (one compiled body serves all
instantiations).

### 2.6 Type inference rules

- `let` bindings infer their type from the initializer; an optional annotation
  `let x: Int = 5` is checked against it.
- Named `fn` declarations **require** parameter type annotations. The return type
  defaults to `Unit` if `->` is omitted.
- Lambda parameters may omit annotations when the lambda appears where a function
  type is expected (e.g. as an argument to `map`), or when unification can determine
  them from the body (`|x| x + 1` infers `fn(Int) -> Int`); if a parameter's type is
  still unknown after checking, it is a compile error.
- Inference is local unification (no let-generalization for lambdas; polymorphism
  comes only from explicit `[T]` parameter lists).
- An unresolved type variable at the end of checking (e.g. `let x = [];` never used)
  is a compile error: "cannot infer type".

---

## 3. Expressions

Fable is expression-oriented. Blocks are expressions; the last expression in a block
(without a trailing `;`) is the block's value. A statement-terminated block has type
`Unit`.

### 3.1 Operators (by precedence, loosest → tightest)

| Level | Operators                | Assoc | Operand types |
|-------|--------------------------|-------|---------------|
| 1     | `\|\|`                   | left  | Bool (short-circuit) |
| 2     | `&&`                     | left  | Bool (short-circuit) |
| 3     | `==` `!=`                | none  | any type T = T (see § 3.2) |
| 4     | `<` `<=` `>` `>=`        | none  | Int, Float, or String (both sides same) |
| 5     | `..` `..=`               | none  | Int |
| 6     | `\|`                     | left  | Int (bitwise or, v0.7) |
| 7     | `^`                      | left  | Int (bitwise xor, v0.7) |
| 8     | `&`                      | left  | Int (bitwise and, v0.7) |
| 9     | `<<` `>>`                | left  | Int (shifts, v0.7) |
| 10    | `+` `-`                  | left  | Int, Float; `+` also String ++ String |
| 11    | `*` `/` `%`              | left  | Int, Float (`%` Int only) |
| 12    | unary `-` `!`            | —     | Int/Float; Bool |
| 13    | call `f(x)`, index `a[i]`, field `.x`, method `.m(x)`, tuple index `.0`, try `?` | left | |

Comparison operators are **non-associative**: `a < b < c` is a parse error.
Integer division truncates toward zero; `/` or `%` by integer zero **panics**.
Float division by zero yields `inf`/`nan` per IEEE-754.

The bitwise operators (v0.7) are Int-only and follow Rust's relative
precedence: shifts bind tighter than `&`, which binds tighter than `^`,
then `\|`, with all four looser than arithmetic and tighter than ranges
and comparisons — so `x & 511 == 0` tests the mask and `1 << n - 1`
shifts by `n − 1`. `>>` is **arithmetic** (sign-extending, matching the
two's-complement Int); `<<` discards bits shifted past the top. A shift
count outside `0..=63` panics. Infix `\|` coexists with lambda syntax
(`\|x\| ...`) because lambdas only begin in operand position. There is no
unary complement: `x ^ -1` flips every bit.

The arithmetic operators (and unary `-`) also apply to user types through
**operator methods** (v0.3, § 5.1): `a + b` dispatches to `a.add(b)` when
`a`'s type defines one. `==`/`!=` remain structural for all types and cannot
be overloaded; compound assignment (`+=`) never dispatches. The bitwise
operators have their own compound-assignment forms (v0.8) — `&= |= ^= <<=
>>=` — Int-only like the plain operators, and likewise never dispatching.

### 3.2 Equality

`==`/`!=` are **structural** (deep) for all types. Both sides must have the same type.
Fields/elements compare in order and stop at the first difference; if two function
values are actually reached during the comparison, it panics at runtime ("cannot
compare functions") — when both operands' types are concretely function-containing,
the checker rejects the comparison statically (E0311). Float equality follows
IEEE-754 (`nan != nan`).

### 3.3 Control flow expressions

```fable
let grade = if score >= 90 { "A" } else if score >= 80 { "B" } else { "C" }
```

- `if` without `else` has type `Unit` (the branch must also be `Unit`).
- `match` is an expression (§ 4).
- `while cond { ... }` and `for pat in iterable { ... }` are statements (they cannot
  appear where a value is required). `for` iterates over a `List[T]` (yielding `T`),
  a `Range` (yielding `Int`), or a `String` (yielding one-character `String`s, by
  Unicode scalar). The loop head takes an **irrefutable pattern** (v0.6): a name,
  `_`, or a (nested) tuple/struct pattern — `for (i, x) in xs.enumerate()`,
  `for (k, v) in m.entries()`, `for _ in 0..3`. Refutable patterns are E0503.
  Loop bindings are fresh immutable bindings each iteration — closures created
  in the body capture that iteration's values. A `for` loop over a
  list iterates the **live** list by index (elements pushed during the loop are
  visited; removed elements are skipped), whereas the callback methods (`each`,
  `map`, ...) iterate a **snapshot** taken when the method is called.
- `break` and `continue` are only valid inside loops. `return expr` / `return`
  exits the enclosing function (top-level `return` is a compile error).
- **Divergence** (v0.6): a block whose last statement is `return`, `break`,
  `continue`, or a `while true { ... }` that contains no `break` takes the type
  demanded by its context — such a block never falls through, so no dead
  trailing expression is required. A function whose body ends in an escape-free
  `while true` loop typechecks with any return type. `panic(..)` and
  `os.exit(..)` likewise typecheck in any value position.
- **`if let` / `while let`** (v0.8): `if let PATTERN = EXPR { .. } [else ..]`
  and `while let PATTERN = EXPR { .. }` test a single pattern against `EXPR`
  without the ceremony of a full `match`. Both are sugar, expanded at parse
  time: `if let` is exactly `match EXPR { PATTERN -> THEN, _ -> ELSE }` (an
  expression, usable anywhere `if` is — with no `else`, the implicit `_`
  branch is `Unit`, so `THEN` must be too, exactly as a plain `if` with no
  `else`); `while let` is exactly `while true { match EXPR { PATTERN ->
  BODY, _ -> break } }` (the drain-loop idiom this replaces). Because both
  desugar to ordinary `match`/`while`, everything about them — pattern
  refutability, `else if let` chaining, `break`/`continue` scoping, GC
  behavior — follows from those forms with no special cases; a pattern that
  happens to be irrefutable (matches unconditionally) makes the implicit
  fallback unreachable, which is silently fine rather than a warning, since
  the user never wrote that arm.

### 3.4 Lambdas

```fable
let double = |x: Int| x * 2
let add = |a, b| a + b           // OK if the context determines the types
nums.map(|n| n * n)
let f = |x: Int| -> Int { x + 1 }   // full form with return type and block
let fill = (0..3).map(|_| 0)     // `_` discards a parameter (v0.6)
```

A parameter written `_` binds nothing and cannot be referenced; several may
appear in one list (`|_, _| 1`).

Lambdas capture variables from enclosing scopes **by reference** (a captured
`let mut` counter shared by two closures is one counter). Captures keep values alive
past the defining scope (closure upvalues).

### 3.5 Indexing

- `xs[i]` on `List[T]`: panics if out of bounds (negative or ≥ len). `xs[i] = v` assigns.
- `m[k]` on `Map[K, V]`: panics if the key is absent (use `m.get(k)` for `Option[V]`).
  `m[k] = v` inserts or overwrites.
- `s[i]` on `String` is **not allowed** (compile error) — use `s.chars()` or `s.slice()`.

---

### 3.6 The `?` operator

`expr?` unwraps a successful `Option`/`Result` or propagates the failure by
returning it from the enclosing function (or lambda):

- If `expr: Option[T]`: `Some(v)?` evaluates to `v`; `None?` returns `None`
  from the enclosing function, whose return type must be an `Option` (its
  payload type is independent of `T`).
- If `expr: Result[T, E]`: `Ok(v)?` evaluates to `v`; `Err(e)?` returns the
  `Err` unchanged, so the enclosing return type must be a `Result` with the
  same error type `E` (its success type is independent of `T`).

`?` binds at postfix precedence (`-x?` is `-(x?)`; `a.f()?.g()?` chains).
Errors: `?` outside a function (E0328), an incompatible enclosing return type
(E0329), a non-`Option`/`Result` operand (E0330).

```fable
fn parse_sum(a: String, b: String) -> Option[Int] {
    Some(a.parse_int()? + b.parse_int()?)
}
```

---

## 4. Pattern matching

```fable
match shape {
    Shape.Circle(r) if r > 100.0 -> "huge circle",
    Shape.Circle(r) -> "circle of radius {r}",
    Shape.Rect(w, h) -> "a {w} by {h} rectangle",
    Shape.Empty -> "nothing",
}
```

- Arms are `pattern [if guard] -> expression` separated by commas (trailing comma OK).
  An arm body may be a block: `pattern -> { stmts }`; after a block-bodied arm the
  comma may be omitted.
- An arm body may also be a bare `return [expr]`, `break`, or `continue`
  (v0.6) — sugar for the one-statement block form, so early exits read
  `None -> return Err("missing"),` without brace ceremony. (Assignment is
  still a statement and still needs a block body: `Some(v) -> { x = v; }`.)
- All arms must have the same type; `match` is an expression. An arm that
  diverges (its body is or ends in `return`/`break`/`continue`, § 3.3)
  unifies with arms of any type.

**Patterns:**

| Pattern            | Example                          |
|--------------------|----------------------------------|
| Literal            | `0`, `"yes"`, `true`, `3.14`     |
| Wildcard           | `_`                              |
| Binding            | `x` (binds the value)            |
| Tuple              | `(a, b, _)`                      |
| Enum variant       | `Some(x)`, `Shape.Rect(w, h)`, `None` — the enum qualifier is optional whenever the scrutinee's type determines the enum |
| Struct             | `Point { x, y }`, `Point { x: 0.0, y }` |
| Or-pattern         | `0 \| 1 \| 2` (all alternatives must bind the same names with the same types) |
| Guard              | `n if n > 0`                     |

- **Exhaustiveness** is checked (Maranget-style usefulness): a non-exhaustive `match`
  is a **compile error** reporting an example of an uncovered pattern.
- An arm that can never match (covered by earlier arms) is an **unreachable-arm
  warning**, not an error. Guarded arms are conservatively treated as possibly failing.
- Int/Float/String literal patterns never make a match exhaustive by themselves
  (a `_`/binding arm is required); `Bool` is exhaustive with `true` and `false`.
- Struct patterns may omit fields only with `..`: `Point { x, .. }`.

`let` destructuring is supported for **irrefutable** patterns only:
`let (a, b) = pair;` and `let Point { x, y } = p;` (a refutable pattern in `let`
is a compile error). `for` loop heads accept the same irrefutable patterns
(§ 3.3): `for (i, x) in xs.enumerate() { ... }`.

---

## 5. Statements and programs

A program is a sequence of items and statements executed top-to-bottom:

```fable
struct/enum declarations     // hoisted, order-independent
fn declarations              // hoisted (mutual recursion OK)
let / let mut bindings       // execute in order
expression statements        // execute in order
```

- `let` bindings at top level are globals; inside blocks they are locals with lexical
  scope and shadowing (`let x = 1; let x = x + 1;` is allowed).
- Assignment `x = expr` requires `x` be declared `let mut` (locals **and** globals).
  Assigning to a plain `let` is a compile error. Field/index assignments (`p.x = v`,
  `xs[i] = v`, `m[k] = v`) do not require `mut` (the binding still points at the same
  object).
- Compound assignment: `x += e`, `-=`, `*=`, `/=`, `%=` (sugar; same rules as
  `=`), plus the bitwise set `&=`, `|=`, `^=`, `<<=`, `>>=` (v0.8, Int-only).
- Semicolons terminate statements. The final expression of a block may omit the
  semicolon to become the block's value. `fn`, `struct`, `enum`, `if`, `match`,
  `while`, `for` used as statements do not need a trailing semicolon.
- Items (`fn`, `struct`, `enum`, `impl`, `import`) may only appear at the top
  level (no nested named functions — use lambdas).

### 5.1 impl blocks (methods)

`impl TypeName { ... }` defines methods on a user-declared struct or enum.
Each method's first parameter is a bare `self` (no type annotation — it has
the impl type); calls use dot syntax on a value of the type.

```fable
struct Point { x: Float, y: Float }

impl Point {
    fn len(self) -> Float { (self.x * self.x + self.y * self.y).sqrt() }
    fn scaled(self, k: Float) -> Point { Point { x: self.x * k, y: self.y * k } }
}

let p = Point { x: 3.0, y: 4.0 };
println(p.scaled(2.0).len());   // 10.0
```

- A generic type's impl re-binds its type parameters by position:
  `impl Pair[A, B] { ... }` — the binder names are local to the block and the
  count must match the declaration (E0332). Methods may add their own
  generics after the impl's.
- Multiple impl blocks per type are allowed; method names must be unique per
  type across all of them (E0333).
- Only user-declared structs and enums can be impl targets — not builtins or
  the prelude `Option`/`Result` (E0331).
- Methods are hoisted like functions (order-independent, mutual recursion
  works) and are ordinary functions with the receiver as argument 0; a method
  travels with its type across modules. In a module, a method is callable
  from outside only if marked `pub` (v0.3, § 5.2).
- **Operator methods** (v0.3): the well-known method names `add`, `sub`,
  `mul`, `div`, `rem`, and `neg` overload `+ - * / %` and unary `-` for the
  type. Dispatch is on the **left** operand's type only, so mixed signatures
  work (`vec * 2.0` calls `fn mul(self, k: Float)`); the right operand and
  result types are whatever the method declares. A binary operator method
  takes exactly one parameter besides `self`; `neg` takes none. Equality
  stays structural, and compound assignment is sugar that never dispatches —
  write `x = x + y`.

### 5.2 Modules and imports

A program may span multiple files. `import a.b;` (top level only) loads
`a/b.fable` **relative to the importing file** and binds it under the alias
`b` — the last path segment — or a chosen name with `import a.b as m;`.

```fable
import geo;                       // loads geo.fable
import util.strings as s;         // loads util/strings.fable

let p = geo.Point { x: 1.0, y: 2.0 };   // qualified struct literal
let d: geo.Point = geo.origin();        // qualified type
println(geo.dist(p, d));                // qualified function call
let f = geo.dist;                       // module functions are values
match shape {
    geo.Shape.Circle(r) -> r,           // qualified variant pattern
    Rect(w, h) -> w * h,                // or type-directed, unqualified
    geo.Shape.Empty -> 0.0,
}
```

- Every module is loaded once (diamond imports share the copy) and its
  top-level code runs once, before any file that imports it; the root file
  runs last. Circular imports are a compile error (E0338); a missing file is
  E0337; duplicate aliases in one file are E0336.
- A module's top-level names are reachable only through its alias — imports
  are not transitive.
- **Visibility** (v0.3): module items are private by default. `pub` on a
  `fn`, `struct`, `enum`, top-level `let`, or an impl-block method exports
  it. The rule is: **naming** a foreign item requires `pub` (qualified
  calls, globals, types, struct literals, qualified patterns, and method
  dispatch across modules — E0339), while **using a value you hold** does
  not (field reads and type-directed variant patterns on a foreign value
  always work). The prelude is public; `pub` in the root module is
  meaningless but harmless. There is no field-level visibility, and the
  checker does not yet flag private types in `pub` signatures.
- `pub` module bindings are readable from outside (`geo.counter`) but
  assignable only inside their own module (E0308).
- Import paths resolve relative to the importing file first, then against
  each directory in the colon-separated `FABLE_PATH` environment variable
  (v0.3) — the home for utility modules shared across projects. The E0337
  error lists every location tried.
- The `std.` prefix is **reserved** (v0.4): `import std.json;` resolves to a
  module embedded in the interpreter binary, never to a file. Embedded
  modules may import only other `std` modules. See § 7.1 for the catalog.
- The REPL imports like a file (v0.5): paths resolve against the working
  directory, then `FABLE_PATH`, then `std.`; loaded modules and aliases
  persist across inputs, and re-importing never reloads. Only one-shot
  string evaluation cannot import (E0334).

---

## 6. Execution model and errors

- **Panics** are runtime errors that abort the program with a message and a stack
  trace: index out of bounds, missing map key via `[]`, integer overflow, integer
  division/modulo by zero, `unwrap()` on `None`/`Err`, `panic("msg")`, failed
  `assert`/`assert_eq`, comparing functions, reading a global before its `let`
  has run, and call-stack overflow (the call depth is capped at 4096 frames).
  Exit codes: success 0, usage error 64, compile error 65, unreadable input 66,
  panic 70.
- **Tail calls are optimized**: a call in tail position — the operand of
  `return`, the final expression of a function or lambda body, or the result
  position of an `if`/`match`/block that is itself in tail position — reuses
  the caller's frame instead of pushing a new one. Tail recursion (direct,
  mutual, or through function values) therefore runs in constant stack space
  and never hits the frame cap; non-tail recursion still overflows.
- The runtime uses a tracing mark-and-sweep garbage collector. Setting the environment
  variable `FABLE_GC_STRESS=1` forces a collection before every allocation (testing);
  `FABLE_GC_LOG=1` logs collections to stderr.

---

## 7. Builtin free functions (prelude)

| Function | Type | Notes |
|----------|------|-------|
| `print(x)` | `[T] fn(T) -> Unit` | writes `str(x)`, no newline |
| `println(x)` | `[T] fn(T) -> Unit` | writes `str(x)` + `\n` |
| `str(x)` | `[T] fn(T) -> String` | display conversion (§ 8.1) |
| `panic(msg)` | `fn(String) -> Unit` | aborts with the message (the call typechecks at any expected type) |
| `assert(cond)` | `fn(Bool) -> Unit` | panics on `false` |
| `assert_eq(a, b)` | `[T] fn(T, T) -> Unit` | panics on inequality, printing both |
| `clock()` | `fn() -> Float` | monotonic seconds |
| `input()` | `fn() -> Option[String]` | reads one line from stdin (no trailing `\n`); `None` at EOF |
| `try(f)` | `[T] fn(fn() -> T) -> Result[T, String]` | runs `f`, catching runtime panics as `Err(message)` (v0.4). Side effects before the panic persist; the VM stack is fully restored. `os.exit` is not catchable. |

Two more builtin namespaces (v0.3) cover the glue-language essentials.
Fallible operations return `Result[_, String]` whose `Err` carries
`"<path>: <OS error>"`, composing with `?`:

| Member | Type | Notes |
|--------|------|-------|
| `fs.read(path)` | `fn(String) -> Result[String, String]` | whole file as UTF-8 |
| `fs.write(path, s)` | `fn(String, String) -> Result[Unit, String]` | create/truncate |
| `fs.append(path, s)` | `fn(String, String) -> Result[Unit, String]` | creates if missing |
| `fs.exists(path)` | `fn(String) -> Bool` | |
| `fs.is_dir(path)` | `fn(String) -> Bool` | |
| `fs.list_dir(path)` | `fn(String) -> Result[List[String], String]` | entry names, sorted |
| `fs.create_dir(path)` | `fn(String) -> Result[Unit, String]` | recursive (`mkdir -p`) |
| `fs.remove(path)` | `fn(String) -> Result[Unit, String]` | file or empty directory |
| `fs.read_bytes(path)` | `fn(String) -> Result[Bytes, String]` | whole file, raw bytes (v0.7) |
| `fs.write_bytes(path, b)` | `fn(String, Bytes) -> Result[Unit, String]` | create/truncate, raw bytes (v0.7) |
| `os.args()` | `fn() -> List[String]` | CLI args after the script path |
| `os.env(name)` | `fn(String) -> Option[String]` | |
| `os.run(cmd, args)` | `fn(String, List[String]) -> Result[(Int, String, String), String]` | `Ok((exit code, stdout, stderr))`; `Err` if the binary can't launch |
| `os.exit(code)` | `[T] fn(Int) -> T` | ends the process immediately; like `panic`, typechecks at any expected type (v0.6) |
| `os.time()` | `fn() -> Float` | Unix-epoch seconds (`clock()` is monotonic) |

Namespaced (no import needed): `math.pi`, `math.e`, `math.sqrt(Float)`,
`math.sin/cos/tan/atan/atan2/log/log2/log10/exp` (Float; `log` is the
**natural** logarithm), `math.pow(Float, Float)`,
`math.fmod(Float, Float) -> Float` (IEEE remainder with the sign of the
dividend — the `%` operator stays Int-only), `math.floor/ceil/round(Float) ->
Float`, `math.abs_int(Int) -> Int`, `math.abs(Float) -> Float`,
`math.min/max(Int, Int) -> Int`, `math.min_float/max_float(Float, Float) ->
Float`, `math.random() -> Float` (uniform [0, 1), xorshift PRNG),
`math.rand_int(lo, hi) -> Int` (uniform in the **inclusive** range; panics if
`lo > hi`), `math.seed(Int) -> Unit`.

`math.seed` scrambles the seed through SplitMix64 before installing it
(v0.6), so nearby seeds produce unrelated streams; the same seed always
reproduces the same stream within a release, but streams are **not** stable
across releases.

The `fft` namespace (v0.7) provides fast Fourier transforms over
split-complex signals — a complex vector is a pair of equal-length
`List[Float]`s (re, im):

| Function | Type | Notes |
|----------|------|-------|
| `fft.fft(re, im)` | `fn(List[Float], List[Float]) -> (List[Float], List[Float])` | forward DFT, no normalization |
| `fft.ifft(re, im)` | `fn(List[Float], List[Float]) -> (List[Float], List[Float])` | inverse DFT, normalized by `1/n` |
| `fft.rfft(x)` | `fn(List[Float]) -> (List[Float], List[Float])` | real input; the first `n/2 + 1` bins of `fft(x, zeros)` |

The derived helper `magnitude(re, im)` — `sqrt(re[i]^2 + im[i]^2)` per
bin — lives in `std.fft` (§ 7.1), which also wraps `rfft`/`ifft` so an
importing file keeps the same spellings. (It was a native in the v0.8
draft, computing `hypot` — which differs from `sqrt(re^2 + im^2)` in the
last ulp, a deviation from this definition; the move to Fable fixed it.)

Any length `n >= 1` is supported in O(n log n): powers of two run an
iterative radix-2 Cooley–Tukey; every other length goes through
Bluestein's chirp-z algorithm. Conventions match `numpy.fft` (forward
sign `e^{-2πikt/n}`, inverse carries the `1/n`); CI cross-checks against
numpy at 1e-9 relative. Zero-length input and mismatched re/im lengths
panic.

The `worker` namespace (v0.7) runs Fable programs as **isolates**: each
worker is a whole separate VM — its own heap, globals, and GC — on its
own OS thread, so workers run in true parallel. Nothing is shared;
the only things that cross the boundary are `String` messages
(structured data goes as JSON by convention, `import std.json`):

| Function | Type | Notes |
|----------|------|-------|
| `worker.spawn(file, args)` | `fn(String, List[String]) -> Result[Worker, String]` | compile + start `file` on a new thread |
| `worker.send(s)` | `fn(String) -> Bool` | worker → parent; errors outside a worker |
| `worker.recv()` | `fn() -> Option[String]` | parent → worker; **blocks**; `None` = parent hung up (joined or dropped the handle); errors outside a worker |
| `worker.try_recv()` | `fn() -> Option[Option[String]]` | (v0.8) the non-blocking twin of `worker.recv()` — never waits; outer `None` = no message ready right now, `Some(None)` = the parent hung up (recv's own terminal state, one level deeper), `Some(Some(s))` = a message; errors outside a worker |
| `worker.is_worker()` | `fn() -> Bool` | is this program running as a worker? |

`spawn` resolves `file` relative to the entry script's directory (the
same rule imports use; absolute paths pass through) and **blocks until
the worker has compiled**, so a missing file or compile error comes back
synchronously as `Err` — a worker never starts half-broken. Inside the
worker, `os.args()` returns the spawn `args`. A worker's panic ends only
its own thread; the parent sees it as `Err` from `join()` (§ 8.4c for
the `Worker` handle methods). Workers may spawn workers. The process
exits when the main script ends — detached workers are not waited for;
`join` what you need. Worker `println` output goes to the same stdout as
the parent (interleaving across threads is unordered; under `fable
test`, worker output is captured with the parent's).

One more free function joined the prelude in v0.6:

| Function | Type | Notes |
|----------|------|-------|
| `char(code)` | `fn(Int) -> String` | the one-character string for a Unicode scalar value; panics on surrogates/out-of-range. Inverse of `code_at` (§ 8.3). |

Two more joined in v0.7:

| Function | Type | Notes |
|----------|------|-------|
| `bytes(n)` | `fn(Int) -> Bytes` | zero-filled byte buffer (§ 8.4b) |
| `bytes_of(xs)` | `fn(List[Int]) -> Bytes` | from byte values 0..255 |

### 7.1 The standard library (v0.4; expanded in v0.7)

Ten modules written in Fable ship inside the interpreter, imported like any
module (`import std.json;`, aliased with `as`). Everything below is `pub`;
these modules follow the same visibility rules as user code.

| Module | Exports |
|--------|---------|
| `std.json` | `parse(String) -> Result[Json, String]`, `stringify(Json) -> String`, `pretty(Json) -> String`; `enum Json { JNull, JBool, JNum, JStr, JArr, JObj }` with methods `get(key)`, `at(i)`, `as_str()`, `as_num()`, `as_bool()`, `is_null()`, `len()`; ergonomic constructors (v0.8) `obj(entries)`, `arr(items)`, `jstr(s)`, `num(f)`, `int(i)` (`= num(i.to_float())`), `bool(b)`, `null()` — the same tree as the raw `Json.J*` constructors, named for what they build (`jstr`, not `str`, so it can't shadow the prelude `str()` for code in this module) |
| `std.flags` | `flag(args, name) -> Bool`, `value(args, name) -> Option[String]` (only `--name=value` carries a value), `value_or(args, name, fallback)`, `positionals(args)`; a literal `--` ends flag parsing |
| `std.path` | `join`, `base_name`, `dir_name`, `ext`, `strip_ext` — purely textual, slash-separated |
| `std.strings` | `lines` (trailing-newline aware), `words`, `join_lines`, `ellipsize`, `strip_prefix`, `strip_suffix`; a `Builder` for string accumulation (v0.7): `builder()` makes one, methods `push(s)`, `push_char(code)`, `len()` (characters so far, O(1)), `is_empty()` (v0.8), `build()` (one join; non-consuming), `push_joined(sep, s)` (v0.8: pushes `sep` first unless this is the builder's first piece — the separator-before-each-line idiom without a manual `if len() > 0` at every call site), `clear()` — `+=` in a loop is O(n²), a Builder is O(n) |
| `std.iter` | lazy sequences: `of(list)`, `count_from(n)`, `from_fn(f)` build an `Iter[T]`; adapters `map`, `filter`, `take`, `chain`, `zip`; consumers `collect`, `fold`, `each`, `count`. Implemented entirely in Fable (an `Iter[T]` is a struct holding a `next` closure). |
| `std.lists` | free-function list helpers (v0.7; builtins cannot gain methods): `fill(n, v)` (`n` aliases of one value), `sum` / `sum_float`, `min` / `max` / `min_float` / `max_float` (`Option`; `None` for `[]`), `min_by(xs, cmp)` / `max_by(xs, cmp)` under the `sort_by` comparator convention (negative/zero/positive), `min_by_key(xs, key)` / `max_by_key(xs, key)` (v0.8: an `Int`-valued key extractor instead of a comparator) — ties keep the **first** winner |
| `std.set` | `Set[T]` (v0.7), backed by `Map[T, Unit]`: structural membership, insertion-order iteration. `new()`, `from_list(xs)` (first occurrence wins); methods `insert(v) -> Bool` / `remove(v) -> Bool` (did anything change), `contains`, `len`, `is_empty`, `to_list`, and `union` / `intersect` / `difference` — each returns a **new** set ordered by the left operand's insertion order, then the right's |
| `std.deque` | `Deque[T]` (v0.7), a double-ended queue with amortized O(1) ends (two-stack representation; a pop on an empty side reverses the other side across). `new()`, `from_list(xs)` (copies); methods `push_front` / `push_back`, `pop_front` / `pop_back` / `front` / `back` (all `Option[T]`), `len`, `is_empty`, `to_list` (front to back) |
| `std.lazy` | `Lazy[T]` (v0.8): deferred, memoized computation. `of(thunk: fn() -> T) -> Lazy[T]` wraps a zero-argument thunk that doesn't run until needed; methods `get() -> T` (computes and caches on the first call, free on every later call — on any reference, since structs are references) and `is_forced() -> Bool`. For a module-level table that's expensive to build and not always needed — a plain top-level `let` already builds once at import (eagerly); `Lazy` defers that to first use. |
| `std.glm` | Vector/matrix/quaternion math (v0.8), named and shaped after GLM: `Vec2`/`Vec3`/`Vec4` (constructors `vec2`/`vec3`/`vec4`; operator methods `add`/`sub`/`neg`; `mul(self, k: Float)`/`div(self, k: Float)` are **scalar** — the one `mul`/`div` slot a type gets (§ 5.1) goes to scaling, matching this spec's own worked example; `dot`, `length`, `length_sq`, `normalize`, `lerp`, and `cross` on `Vec3`); `Mat4` (column-major, `c0`..`c3`; constructors `mat4_identity`, `translation`, `scaling`, `rotation_x`/`y`/`z`, `rotation_axis` (Rodrigues', axis normalized internally), `perspective`/`ortho`/`look_at` (right-handed, OpenGL NDC z in `[-1, 1]`); methods `mul(self, o: Mat4)` (composition — chain as `proj.mul(view).mul(model)`), `mul_vec4(self, v: Vec4)` (the transform apply, named since `mul`'s operator slot is taken by composition), `transpose`); `Quat` (constructors `quat`, `quat_identity`, `from_axis_angle`; methods `mul` (composition), `conjugate`, `normalize`, `length`, `to_mat4`, `slerp` — computed via `atan2`/`sqrt` since `math` has no `acos`). Pure Fable, no native code. |
| `std.fft` | FFT helpers (moved from the native namespace in the minification pass): `magnitude(re, im)` (`sqrt(re[i]^2 + im[i]^2)` per bin, exactly as written — the panic on length mismatch matches the old native's message byte for byte), plus one-line `rfft`/`ifft` wrappers so an importing file keeps the `fft.` spellings (an imported module shadows the builtin namespace). `fft.fft` (complex pairs) is deliberately not re-exported — a module fn named `fft` would shadow the namespace in this module's own bodies; files that need it use the native namespace and skip this import. |

### 7.2 The gpu namespace (v0.7, experimental, feature-gated)

The `gpu` namespace dispatches compute shaders. It has five backends,
all **native and zero-dependency** (since v0.8): **Metal** (MSL kernels;
raw FFI, behind the `metal` feature on Apple Silicon macOS, the same
feature as the Metal window backend), **Vulkan** (SPIR-V binaries via
`gpu.run_spirv`; raw `dlopen` FFI, behind the `vulkan` feature on
Linux/Windows), **OpenCL** (SPIR-V binaries via `gpu.run_spirv` in
the *OpenCL profile* — see below; raw `dlopen` FFI over the ICD loader,
behind the `opencl` feature on Linux/Windows; requires an OpenCL 2.1+
runtime with IL ingestion at run time), **CUDA** (PTX kernels via
`gpu.run` — PTX is textual, so it rides the `String` argument like MSL;
raw `dlopen` FFI to NVIDIA's driver, behind the `cuda` feature on
Linux/Windows; no CUDA toolkit involved, the driver JITs the PTX), and
**Direct3D 12** (HLSL kernels via `gpu.run`, compiled at dispatch time
by the OS's own `d3dcompiler_47.dll`; COM-vtable FFI over runtime-loaded
system DLLs, behind the `d3d12` feature on Windows — where WARP, the
OS's software adapter, guarantees a device on every machine). Where
several are compiled in, the precedence is vulkan > d3d12 > cuda >
opencl (the CI-proven universal path first, then the always-has-a-device
Windows path, then the vendor GPU path, then OpenCL, which commonly
resolves to a CPU implementation). The original **wgpu** path (WGSL
shaders behind a `gpu` cargo feature — v0.7's one quarantined
dependency) was **removed in v0.8** when this native coverage landed,
per `CLAUDE.md`'s roadmap: every build of Fable is now zero-dependency
(CI asserts `cargo tree` is a single line for the default and for every
feature set). The namespace itself always exists — programs using it
typecheck and run in every build; without a backend the members degrade
gracefully as described below.

| Member | Type | Notes |
|--------|------|-------|
| `gpu.available()` | `fn() -> Bool` | is a GPU adapter usable? Always `false` without a backend |
| `gpu.adapter_info()` | `fn() -> String` | `"<name> (<backend>)"`, `"no adapter"`, or `"gpu support not compiled in"`. Never empty |
| `gpu.run(src, input, out_len, wx, wy, wz)` | `fn(String, Bytes, Int, Int, Int, Int) -> Result[Bytes, String]` | one compute dispatch of source-text `src` — MSL on the metal backend, PTX on the cuda backend, HLSL on the d3d12 backend (the ABIs below). The binary backends (vulkan, opencl) `Err` redirecting to `gpu.run_spirv`. Without a backend: `Err("gpu support not compiled in (build with --features metal on Apple Silicon macOS, --features d3d12 on Windows, or --features vulkan, cuda, or opencl on Linux/Windows)")` |
| `gpu.run_spirv(spirv, input, out_len, wx, wy, wz)` | `fn(Bytes, Bytes, Int, Int, Int, Int) -> Result[Bytes, String]` | (v0.8) `gpu.run`'s `Bytes`-shader sibling — SPIR-V is a binary format, so the blob rides the buffer type (a sibling, not an overload: Fable has neither default parameters nor overloading). Ingested natively by the vulkan and opencl backends — each in its own SPIR-V *profile* (the two ABI paragraphs below; the blobs are not interchangeable); other backends `Err` naming the entry point they want |
| `gpu.backend()` | `fn() -> String` | (v0.8) `"metal"`, `"vulkan"`, `"d3d12"`, `"cuda"`, `"opencl"`, or `"none"` — which implementation the `gpu` namespace dispatches to in this build (vulkan > d3d12 > cuda > opencl). The `gpu` analog of `win.backend_name()`: branch on it to pick the kernel dialect and entry point — and, for `gpu.run_spirv`, the SPIR-V profile |

Every failure is an `Err` value, never a panic: bad arguments, no adapter,
shader compile/validation errors (their messages pass through), device loss.

**The shared kernel contract.** Whatever the backend's dialect, every
kernel sees the same two buffers: an input initialized with the argument
bytes (which must be non-empty and a multiple of 4 bytes), and an output
of `out_len` bytes (positive, a multiple of 4, at most 256 MiB),
zero-initialized and returned as the `Ok` value after the dispatch covers
the `(wx, wy, wz)` index space (each count in `1..=65535`). Byte order is
the GPU's little-endian layout. Argument validation is shared, so bad
calls fail with byte-identical messages in every build.

**The MSL ABI (the native Metal backend, v0.8)** expresses that contract
in MSL:

```msl
#include <metal_stdlib>
using namespace metal;
kernel void compute_main(device const uint* input  [[buffer(0)]],
                         device uint*       output [[buffer(1)]],
                         uint3 gid [[thread_position_in_grid]]) { ... }
```

The entry point is named `compute_main` (MSL reserves `main`; this matches
the `gfx` backend's `vertex_main`/`fragment_main` convention). The
dispatch is `(wx, wy, wz)` threadgroups of **one thread each**, so
`thread_position_in_grid` covers exactly the `(wx, wy, wz)` index space —
larger threadgroups are an API-side parameter in Metal rather than a
shader-side declaration, and would be a new explicit argument if ever
needed, not a silent change. The worked example is
`docs/assets/metal_compute.fable` (an MSL doubling kernel, which
hard-asserts its output bytes whenever a device exists — compute needs no
window server, so it is a real correctness gate on any Metal-capable
machine).

**The SPIR-V ABI (the native Vulkan backend, v0.8)** is the same contract
again, expressed in SPIR-V: `gpu.run_spirv`'s first argument is the
binary (a `Bytes` blob of 4-byte words, magic `0x07230203`), whose module
must declare a `GLCompute` entry point named `main` (SPIR-V reserves
nothing; every GLSL toolchain emits `main`) with two `BufferBlock`
storage buffers at descriptor set 0, bindings 0/1, and its own
`LocalSize` execution mode — the dispatch is `(wx, wy, wz)` workgroups of
whatever size the module declares. The worked example is
`docs/assets/vulkan_compute.fable` — a hand-assembled doubling kernel,
hard-asserted whenever a compute device exists; Mesa's lavapipe software
device makes that unconditional on CI, the first `gpu` backend fully
exercised without GPU hardware.

**The SPIR-V ABI (the native OpenCL backend, v0.8)** is the same contract
through the same entry point — but SPIR-V is the roadmap's lingua-franca
*format* (`CLAUDE.md`), and compute kernels come in two **profiles** the
format does not paper over. A Vulkan-profile module declares a
`GLCompute` entry point under `Logical` addressing and the `GLSL450`
memory model, with buffers as descriptor-set storage buffers; an
OpenCL-profile module declares a `Kernel` entry point under `Physical64`
addressing and the `OpenCL` memory model, with buffers as
`CrossWorkgroup` pointer *kernel arguments*. `clCreateProgramWithIL`
rejects the former and Vulkan rejects the latter: **the blob passed to
`gpu.run_spirv` must match the active backend's profile, and
`gpu.backend()` is the branch point** — the same rule that already picks
GLSL vs. MSL for source-text shaders. On the opencl backend the
module must name its `Kernel` entry point `main` and take exactly two
`CrossWorkgroup` pointer parameters: argument 0 is bound to the input
bytes (read-only), argument 1 to `out_len` zero-initialized bytes
returned after the dispatch. `(wx, wy, wz)` is the **global work size**
(total work-items, local size left to the implementation), which covers
exactly the index space a Vulkan module with `LocalSize 1 1 1` dispatches
— `GlobalInvocationId` (OpenCL C's `get_global_id`) agrees across both
profiles. At run time the backend needs an OpenCL 2.1+ runtime whose
device advertises SPIR-V ingestion (`CL_DEVICE_IL_VERSION`); errors name
what is missing, and shader build failures carry the runtime's build log.
The worked example is `docs/assets/opencl_compute.fable` — the
OpenCL-profile twin of `vulkan_compute.fable`, hard-asserting the same
doubled bytes (a CPU implementation like pocl counts; no GPU hardware
needed).

**The PTX ABI (the native CUDA backend, v0.8)** expresses the contract in
NVIDIA's textual virtual ISA, which the driver JITs for the resident GPU
at module load — so it travels through `gpu.run`'s `String` argument,
like MSL. The module must declare `.visible .entry main` taking exactly
two `.param .u64` pointer parameters (input, then output; convert with
`cvta.to.global.u64` before global loads/stores). The launch is a
`(wx, wy, wz)` **grid of single-thread blocks**, so `%ctaid` spans the
index space — the same shape as the Metal dispatch. `gpu.run_spirv` on
this backend `Err`s redirecting to `gpu.run`: the driver API has no
SPIR-V ingestion. The worked example is `docs/assets/cuda_compute.fable`
(a hand-written PTX doubling kernel, `.version 6.0`/`.target sm_50` for
broad JIT compatibility). Honesty about verification: no software CUDA
implementation exists, so unlike the other three backends this one is
exercised end to end only on real NVIDIA hardware — CI pins the graceful
no-driver error path, and the battery hard-asserts its bytes the first
time a GPU runs it.

**The HLSL ABI (the native Direct3D 12 backend, v0.8)** expresses the
contract in HLSL, compiled to DXBC at dispatch time by
`d3dcompiler_47.dll` — an OS component, so the compiler adds no
dependency. The kernel must declare `void main` (HLSL reserves nothing)
with the two buffers as `RWByteAddressBuffer`s at `u0` (input) and `u1`
(output), bound as *root UAVs* — no descriptor tables anywhere — and a
`[numthreads]` attribute of its choosing; the dispatch is `(wx, wy, wz)`
**thread groups**, so with `[numthreads(1,1,1)]` `SV_GroupID` spans the
index space, the same shape as the Metal and CUDA dispatches.
`gpu.run_spirv` on this backend `Err`s redirecting to `gpu.run` (D3D12
ingests DXBC/DXIL, not SPIR-V). Uniquely on Windows, availability is
unconditional: WARP, the OS's software D3D12 adapter, provides a device
on any Windows 10+ machine, GPU or none — which is why the worked
example, `docs/assets/d3d12_compute.fable`, is hard-asserted end to end
on plain CI runners, like the Vulkan (lavapipe) and OpenCL (Intel CPU
runtime) batteries.

`gpu.run`'s I/O rides on the `Bytes` buffer type (§ 8.4b), which ships in
every build — `bytes(n)`/`bytes_of(..)` construct the input, and the LE
pushers (`push_u32le`, ...) / `to_list()` bridge to and from numeric data.

### 7.3 The window namespace (v0.8, Linux + Windows + macOS (Apple Silicon), feature-gated)

The `window` namespace is the GLFW-equivalent piece of the native-OpenGL
roadmap (`std.glm`, § 7.1, shipped the math side): window creation, event
polling, keyboard/mouse state, and a trivial clear-color + swap-buffers —
enough to prove the whole pipe end-to-end. This namespace is deliberately
just the GLFW-shaped part; the backend-neutral GL draw-call layer built on
top of it is the separate `gfx` namespace (§ 7.4).

Like `gpu`, its implementation is quarantined behind a cargo feature — here
`gl` — but unlike `gpu`, the feature adds **zero** Cargo dependencies: it is
raw FFI to system libraries (X11 linked normally; GL/GLX resolved with
`dlopen`/`dlsym` at runtime, since GL *dev* packages are far less reliably
preinstalled than X11's), so `cargo tree` stays a single line with or
without it. The namespace itself always exists — programs using it typecheck
in every build; without the feature (or on a platform without a backend —
Windows, Linux, and Apple Silicon macOS are covered; **x86_64 macOS is not
and has no plan to be** (see the macOS backend note below for why)),
`window.create` degrades to `Err`, the same way `gpu.run` does without
`gpu`.

| Member | Type | Notes |
|--------|------|-------|
| `window.create(title, w, h)` | `fn(String, Int, Int) -> Result[Window, String]` | opens an OS window of size `w`×`h` with a current GL context. Without the feature (or backend): `Err("windowing support not compiled in (build with --features gl)")` |
| `window.create_metal(title, w, h)` | `fn(String, Int, Int) -> Result[Window, String]` | (v0.8, macOS/Apple Silicon only) opens a Metal-backed window — a sibling of `create`, additive alongside it, never a replacement (see the macOS Metal backend note below). Without the feature (or off Apple Silicon macOS): `Err("Metal windowing support not compiled in (build with --features metal, aarch64-apple-darwin only)")` |
| `window.create_vulkan(title, w, h)` | `fn(String, Int, Int) -> Result[Window, String]` | (v0.8) opens a Vulkan-backed window — `create_metal`'s Linux/Windows analog, riding the same `vulkan` cargo feature as `gpu.run_spirv` (see the Linux Vulkan backend note below). Implemented on Linux/X11 and Windows (`VK_KHR_xlib_surface` / `VK_KHR_win32_surface`), at full `gfx.*` parity on both — everything past the surface is one shared backend (`window/vulkan.rs`), so the two platforms are behaviorally identical. Without the feature (or off Linux/Windows): `Err("Vulkan windowing support not compiled in (build with --features vulkan, Linux/X11 or Windows)")`; without a display or Vulkan device at runtime, a prefixed `Err` naming the failing step |

`Window` is a nameable opaque type (§ 8.4d) — the handle `create` returns on
success.

### window namespace, Windows backend

The Windows/WGL backend (`src/window/win32.rs`) has the same zero-Cargo-
dependency shape as Linux/X11/GLX, but a simpler linking story: `user32`,
`gdi32`, and `opengl32` ship on every Windows install, so they're linked
normally, with no `dlopen`/`LoadLibrary` dance needed for GL entry points
(unlike Linux, where GL dev packages vary enough to make dynamic resolution
the safer default). Event handling is callback-driven (`WNDPROC` +
`GWLP_USERDATA`) rather than poll-based like Xlib, but this is purely an
internal implementation detail — `poll()`, `key_down()`, `mouse_pos()`,
`width()`/`height()`, and `should_close()` behave identically to the Linux
backend from Fable's point of view. `key_down` names on Windows are a small
hand-written table (ASCII letters/digits plus common named keys like
`"space"`/`"escape"`/`"left"`) rather than X11 keysym names, but the common
single-character and named-key spellings used in practice (`"w"`, `"a"`,
`"space"`, `"escape"`, arrow keys) work the same on both platforms.

### window namespace, macOS backend (Apple Silicon only)

The macOS backend (`src/window/macos/gl.rs`) targets `aarch64-apple-darwin`
only — **x86_64 Macs are not supported and none is planned**: an x86_64
`objc_msgSend` call site must pick between the normal entry point and the
separate `objc_msgSend_stret` one whenever the returned struct doesn't fit
in registers, while Apple's arm64 ABI returns small aggregates (including
the one struct-returning message this backend sends, `NSRect`) directly
from plain `objc_msgSend`, with no `_stret` variant existing in the arm64
SDK at all — one dispatch path to get right instead of two, and it matches
the release matrix's `aarch64-apple-darwin`-only macOS target already. It
links `Cocoa` (AppKit + Foundation) and the Objective-C runtime normally —
both ship on every Mac — and resolves `OpenGL.framework`'s two `gl*` draw
calls (`glClearColor`/`glClear`) via `dlopen`/`dlsym`, the same dynamic-
resolution strategy the Linux backend uses for `libGL.so.1`; window/context
creation itself goes through Cocoa's `NSWindow`/`NSOpenGLPixelFormat`/
`NSOpenGLContext`, messaged via `objc_msgSend`. Event handling is poll-based
like Xlib (`[NSApp nextEventMatchingMask:...untilDate:[NSDate distantPast]]`
drains everything queued, once per frame, never blocking) rather than
callback-driven like Win32. `key_down` names on macOS come from
`NSEvent.charactersIgnoringModifiers` (text, not a hardware-independent
keysym or scancode) — the common single-character and space-bar spellings
used in practice (`"w"`, `"a"`, `" "`) work the same as on Linux/Windows,
but this is layout/shift-sensitive in a way X11's keysym-name lookup is
not. Close-button detection has no `NSWindowDelegate` callback installed
(building one requires registering a runtime Objective-C class, which this
poll-per-frame API deliberately avoids); instead `should_close` is set once
`[window isVisible]` goes false after a click on the close box, which
AppKit's default close path guarantees without a delegate.

### window namespace, macOS Metal backend (v0.8, Apple Silicon only, additive)

`window.create_metal` opens a Metal-backed window as a **sibling** to
`create`'s OpenGL/CGL path (`src/window/macos/gl.rs`) — additive, never a
replacement, per this project's standing exception for Metal on macOS (see
`CLAUDE.md`'s engineering principles): both backends compile into the same
binary under `--features gl,metal`, quarantined behind their own `metal`
cargo feature with the same zero-Cargo-dependency shape `gl` already has,
and a program picks per-window which one it wants by calling `create` or
`create_metal`. A sibling entry point rather than a `backend` parameter on
`create`: Fable has neither default parameters nor overloading, so a
mandatory extra argument would break every existing `window.create(title,
w, h)` call site for no ergonomic gain.

`Window.backend_name()` (§ 8.4d) reports which backend a given window is
running — the one place a Fable program needs to branch when targeting
both, since shader *source text* is inherently backend-specific (GLSL vs.
Metal Shading Language); every other `gfx`/`Window` member has the same
call shape and behavior regardless of backend.

Internally, `src/window/macos/mod.rs`'s `Inner` is a small enum
(`Gl(gl::Inner)` / `Metal(metal::Inner)`, each gated on its own cargo
feature) rather than the plain single-backend struct `win32` uses — the
only way one compiled binary can transparently hold either kind of live
window. (`x11` adopted the same enum shape when its Vulkan sibling
arrived — see the Linux Vulkan backend note below.)

`create_metal` opens a real window (`MTLDevice` + command queue +
`CAMetalLayer` hosted by the content view), and the entire `Window` +
`gfx.*` surface behaves identically to the OpenGL backend — rendering
lands in an app-owned offscreen texture that `swap_buffers` blits into the
frame's drawable and presents (see `metal.rs`'s module docs for why the
offscreen indirection is load-bearing). Environments with no Metal-capable
GPU degrade gracefully: `Err("window.create_metal:
MTLCreateSystemDefaultDevice returned nil ...")`.

The `gfx.*` calls have the same observable semantics on both backends —
including Y-origin normalization (`viewport` and `read_pixels` flip
internally between GL's bottom-left and Metal's top-left origins, and
`read_pixels` returns bottom-up RGBA rows on both, so the same call reads
the same physical pixels) — with shader **source text** as the one
deliberate per-backend difference (`win.backend_name()`, § 8.4d). The
Metal shader conventions:

- `compile_program`'s two sources are each standalone MSL whose entry
  functions are named `vertex_main` and `fragment_main` (the analog of
  GLSL's fixed per-stage `main`).
- Vertex attributes arrive via `[[stage_in]]` with `[[attribute(i)]]`
  matching `gfx.set_vertex_attrib`'s index; shaders never name vertex
  buffer indices (the backend binds attribute `i`'s data at `1 + i`
  internally).
- Cross-stage varyings (vertex outputs the fragment stage reads) carry an
  explicit `[[user(name)]]` semantic **in both stage structs**: the two
  sources are separate MSL translation units, and the semantic name — not
  struct-member order — is what reliably links them across separately
  compiled functions. (And assign every varying in the vertex function:
  MSL does not error on an uninitialized output member — it rasterizes as
  undefined data. See `demos/glcube/main_metal.fable` for the worked
  example.)
- Each stage's uniforms live in one struct argument at `[[buffer(0)]]`;
  `gfx.set_uniform_*` resolves the member by *name* via pipeline
  reflection, and names a shader doesn't declare are silently ignored —
  both exactly like GLSL uniform locations.
- Textures are `[[texture(unit)]]` with the unit from
  `gfx.active_texture_unit`; sampling state is declared in the shader as a
  `constexpr sampler` (the MSL spelling of the fixed linear/clamp-to-edge
  mode `upload_texture` configures on GL).
- One clip-space caveat travels with GL-convention projection matrices
  (`std.glm.perspective`): GL puts clip-space z in `[-w, +w]`, Metal in
  `[0, +w]`, so an MSL vertex shader using such a matrix remaps once at
  the end — `out.position.z = (out.position.z + out.position.w) * 0.5;`
  — or half the depth range is clipped away. See
  `demos/glcube/main_metal.fable`, whose golden pins are byte-identical
  to the OpenGL `main.fable`'s (the cross-backend pixel-parity proof,
  asserted in CI on real Apple Silicon hardware).

### window namespace, Vulkan backend (v0.8, Linux + Windows, additive)

`window.create_vulkan` is the Linux/Windows analog of `create_metal`: a **sibling**
to `create`'s OpenGL/GLX path, additive alongside it, never a replacement,
riding the same zero-Cargo-dependency `vulkan` cargo feature the
`gpu.run_spirv` compute backend (§ 7.2) ships under — both backends compile
into the same binary under `--features gl,vulkan`, and a program picks
per-window which one it wants by calling `create` or `create_vulkan`.
Internally, `src/window/x11/`'s `Inner` adopted `macos/`'s two-variant enum
shape (`Gl(gl::Inner)` / `Vulkan(vulkan::Inner)`, each gated on its own
cargo feature) over a shared `X11WindowState` (Xlib window creation + event
pump, written once in `x11/shared.rs`).

`create_vulkan` opens a real window: a WSI swapchain
(`VK_KHR_surface`/`VK_KHR_xlib_surface`/`VK_KHR_swapchain`, FIFO) over the
X window, with all rendering landing in an app-owned offscreen image that
`swap_buffers` copies into the frame's acquired swapchain image and
presents (the same stable-back-buffer indirection as the Metal backend,
for the same reasons — see `window/vulkan.rs`'s module docs; that file
is the platform-neutral core shared by the Linux and Windows Vulkan
backends, with `x11/vulkan.rs` and `win32/vulkan.rs` as thin
surface-creation shims over it). The backend
prefers a UNORM surface format explicitly (`B8G8R8A8_UNORM`, then
`R8G8B8A8_UNORM`) so clear values stay linear — lavapipe offers an sRGB
format first, which would silently re-encode every color. Presentation is
verified with real pixels: a unit test clears the back buffer, presents,
and reads the exact color back out of the X window via `XGetImage` (CI
runs it against Mesa's lavapipe under Xvfb — like the Vulkan compute
backend, this one is fully exercised on plain CI hardware, no GPU
needed). Environments without a display or Vulkan device degrade to a
clean prefixed `Err`.

The full `gfx.*` draw-call surface works on this backend, with **SPIR-V
binaries** as the shader input (`gfx.compile_program_spirv`, § 7.4 —
Vulkan has no runtime GLSL compiler, and shipping one would break the
zero-dependency invariant; this is the same lingua-franca decision
`gpu.run_spirv` made for compute). `gfx.compile_program` (GLSL source)
returns a clean `Err` redirecting to `compile_program_spirv`, and vice
versa on the source-text backends — `win.backend_name()` is the branch
point, exactly as GLSL-vs-MSL already is on macOS. The Vulkan shader
conventions:

- Both modules name their entry point `main` (vertex and fragment are
  separate SPIR-V modules, so the names don't collide).
- Each stage's uniforms live in one `Block`-decorated uniform-buffer
  struct at descriptor set 0 — binding 0 for the vertex stage, binding 1
  for the fragment stage, by convention (the backend reads the actual
  binding from the module's decorations). `gfx.set_uniform_*` resolves
  the member by *name* via the backend's in-house SPIR-V reflection
  (`OpMemberName` + `OpMemberDecorate Offset` over the instruction
  words), so the module must carry member names (glslang and friends
  emit them by default); names a shader doesn't declare are silently
  ignored — both exactly like GLSL uniform locations.
- Vertex attributes are `Input` variables whose `Location` matches
  `gfx.set_vertex_attrib`'s index; shaders never name vertex buffer
  bindings (the backend binds attribute *i*'s data at binding *i*
  internally). Textures are combined image samplers at set 0 binding
  `2 + unit`, with the unit from `gfx.active_texture_unit`.
- Y-axis needs **no** shader-side handling: the backend renders with a
  maintenance1 negative-height viewport, so clip-space +Y is up exactly
  as in GL. The one clip-space caveat that does travel with
  GL-convention projection matrices (`std.glm.perspective`) is depth:
  GL puts clip z in `[-w, +w]`, Vulkan in `[0, +w]`, so a vertex shader
  using such a matrix remaps once at the end —
  `pos.z = (pos.z + pos.w) * 0.5;` — the same remap the Metal backend
  documents (§ 7.3 above).

Verified with real rasterized pixels on CI, twice over:
`docs/assets/vulkan_triangle.fable` draws a hand-assembled SPIR-V
triangle and hard-asserts the exact center pixel through
`gfx.read_pixels`, and `demos/glcube/main_vulkan.fable` renders the
spinning-cube demo with golden pins **byte-identical** to the OpenGL
`main.fable`'s and the Metal `main_metal.fable`'s — the same Fable
program rendering the same pixels on three graphics APIs — both under
Xvfb + lavapipe (no GPU needed).

### 7.4 The gfx namespace (v0.8, feature-gated)

`gfx` is a backend-neutral OpenGL 3.3 core-profile draw-call layer on top of
`window`'s per-platform GL function-pointer table (§ 7.3): shaders,
programs, buffers, vertex arrays, textures, uniforms, and draw calls.
Rather than taking a `Window` receiver per call, every `gfx.*` member
operates against "whichever window is currently current" — the same
single-current-context model `glfwMakeContextCurrent` uses. `Window` gained
one more method for this (§ 8.4d): `make_current() -> Unit` marks a window
as the one every subsequent `gfx.*` call targets, and is idempotent — the
same make-current call `clear()`/`swap_buffers()` already issue internally
per call, just exposed on its own.

Like `window`, `gfx` is quarantined behind the rendering-backend cargo
features (`gl`, `metal`, `vulkan` — it is compiled in whenever any of
them is) and adds **zero** Cargo dependencies (raw FFI only); the
namespace always exists — programs using it typecheck in every build.
Every member degrades gracefully: without any rendering backend, every
`gfx.*` call panics with
`"gfx support not compiled in (build with --features gl, metal, or vulkan)"`;
with one but before any window has ever called `make_current()`, every call
panics with `"gfx: no current GL context -- call window.make_current()
first"`. The one exception is `compile_program`, whose failures — including
both of the above — are `Err` values instead of panics, matching its
`Result` return; every other member "assumes valid GL state" once a
program is validly linked and bound, the same no-`Result`-plumbing shape
`Window`'s own methods use (§ 8.4d).

| Member | Type | Notes |
|---|---|---|
| `gfx.compile_program(vertex_src, fragment_src)` | `fn(String, String) -> Result[Int, String]` | compiles + links a GLSL vertex/fragment pair; `Err` carries the driver's shader/link info log, sized via `GL_INFO_LOG_LENGTH` (never a guessed fixed buffer) — and any shader/program object already created is deleted before returning. On the Vulkan backend: a clean `Err` redirecting to `compile_program_spirv` |
| `gfx.compile_program_spirv(vertex, fragment)` | `fn(Bytes, Bytes) -> Result[Int, String]` | (v0.8) the Vulkan backend's shader input: two SPIR-V binaries (vertex, fragment — see the Linux Vulkan backend note in § 7.3 for the module conventions). A sibling of `compile_program`, not an overload (Fable has neither): SPIR-V is a binary format, so it rides `Bytes`, exactly like `gpu.run_spirv`. On source-text backends (GL, Metal): a clean `Err` redirecting to `compile_program` |
| `gfx.use_program(p)` | `fn(Int) -> Unit` | `glUseProgram` |
| `gfx.delete_program(p)` | `fn(Int) -> Unit` | `glDeleteProgram` |
| `gfx.create_buffer()` | `fn() -> Int` | `glGenBuffers(1, ...)` |
| `gfx.delete_buffer(b)` | `fn(Int) -> Unit` | `glDeleteBuffers(1, ...)` |
| `gfx.bind_buffer(kind, b)` | `fn(String, Int) -> Unit` | `kind` is `"vertex"` or `"index"` → `GL_ARRAY_BUFFER` / `GL_ELEMENT_ARRAY_BUFFER` |
| `gfx.upload_buffer(kind, data, dynamic)` | `fn(String, Bytes, Bool) -> Unit` | `glBufferData` on `kind`'s currently bound buffer; `dynamic` picks `GL_DYNAMIC_DRAW` over `GL_STATIC_DRAW` |
| `gfx.create_vertex_array()` | `fn() -> Int` | `glGenVertexArrays(1, ...)` |
| `gfx.bind_vertex_array(v)` | `fn(Int) -> Unit` | `glBindVertexArray` |
| `gfx.delete_vertex_array(v)` | `fn(Int) -> Unit` | `glDeleteVertexArrays(1, ...)` |
| `gfx.set_vertex_attrib(index, size, stride, offset)` | `fn(Int, Int, Int, Int) -> Unit` | `glVertexAttribPointer` (fixed to `GL_FLOAT`, `normalized = false` — `f32` vertex data only, v1 scope) + `glEnableVertexAttribArray`; `stride`/`offset` are byte counts |
| `gfx.disable_vertex_attrib(index)` | `fn(Int) -> Unit` | `glDisableVertexAttribArray` |
| `gfx.create_texture()` | `fn() -> Int` | `glGenTextures(1, ...)` |
| `gfx.delete_texture(t)` | `fn(Int) -> Unit` | `glDeleteTextures(1, ...)` |
| `gfx.bind_texture(t)` | `fn(Int) -> Unit` | `glBindTexture(GL_TEXTURE_2D, t)` |
| `gfx.active_texture_unit(unit)` | `fn(Int) -> Unit` | `glActiveTexture(GL_TEXTURE0 + unit)` |
| `gfx.upload_texture(data, width, height, has_alpha)` | `fn(Bytes, Int, Int, Bool) -> Unit` | `glTexImage2D` onto the bound `GL_TEXTURE_2D` (`GL_RGBA`/`GL_RGB` by `has_alpha`, always `GL_UNSIGNED_BYTE`); also sets `GL_LINEAR` filtering and `GL_CLAMP_TO_EDGE` wrapping (fixed defaults, v1 scope) |
| `gfx.set_uniform_int(p, name, v)` | `fn(Int, String, Int) -> Unit` | binds `p`, then `glUniform1i` |
| `gfx.set_uniform_float(p, name, v)` | `fn(Int, String, Float) -> Unit` | `glUniform1f` |
| `gfx.set_uniform_vec2(p, name, x, y)` | `fn(Int, String, Float, Float) -> Unit` | `glUniform2f` |
| `gfx.set_uniform_vec3(p, name, x, y, z)` | `fn(Int, String, Float, Float, Float) -> Unit` | `glUniform3f` |
| `gfx.set_uniform_vec4(p, name, x, y, z, w)` | `fn(Int, String, Float, Float, Float, Float) -> Unit` | `glUniform4f` |
| `gfx.set_uniform_mat4(p, name, m)` | `fn(Int, String, Mat4) -> Unit` | flattens `std.glm`'s `Mat4` (§ 7.1: 4 `Vec4` columns `c0..c3`, itself already column-major) into 16 `f32`s; `glUniformMatrix4fv(..., transpose=GL_FALSE, ...)` |
| `gfx.draw_arrays(first, count)` | `fn(Int, Int) -> Unit` | `glDrawArrays(GL_TRIANGLES, first, count)` |
| `gfx.draw_elements(count, byte_offset)` | `fn(Int, Int) -> Unit` | `glDrawElements(GL_TRIANGLES, count, GL_UNSIGNED_INT, byte_offset)` — `u32` indices fixed; `byte_offset` into the bound element-array buffer |
| `gfx.clear(r, g, b, a)` | `fn(Float, Float, Float, Float) -> Unit` | `glClearColor` + `glClear(GL_COLOR_BUFFER_BIT \| GL_DEPTH_BUFFER_BIT)` — unlike `Window.clear` (§ 8.4d, color only), also clears depth |
| `gfx.set_depth_test(enabled)` | `fn(Bool) -> Unit` | `glEnable`/`glDisable(GL_DEPTH_TEST)` |
| `gfx.viewport(x, y, w, h)` | `fn(Int, Int, Int, Int) -> Unit` | `glViewport` |
| `gfx.read_pixels(x, y, w, h)` | `fn(Int, Int, Int, Int) -> Bytes` | `glReadPixels(..., GL_RGBA, GL_UNSIGNED_BYTE, ...)` into a freshly allocated `w * h * 4`-byte buffer — for pixel-spot-check golden tests, this repo's verification convention for rendered output (rather than full-framebuffer hashing) |

`gfx.set_uniform_mat4`'s `m` parameter is typed as a fresh scheme variable
in the native signature, not a concrete `Mat4` — natives can't reference a
`std` module struct's `DefId`, which isn't assigned until `std.glm` is
imported. Passing anything other than an actual `Mat4` (4 `Vec4` fields,
each 4 `Float`s) is therefore a runtime panic (a clear message naming the
expected shape), not a compile error.

Every GL object handle this namespace hands back or takes (programs,
buffers, vertex arrays, textures — shaders are internal to
`compile_program` and never surfaced) is a plain `Int`, matching every
other GL-object convention in this namespace; there is no dedicated handle
type.

---

## 8. Builtin methods

Method calls are type-directed (resolved at compile time from the receiver's type).

### 8.1 Display conversion — `str(x)` / interpolation

- `Int` → `42`; `Float` → shortest round-trip digits, in positional notation with a
  decimal point (`3.0`, `0.1`) for magnitudes in `[1e-4, 1e16)` and for zero, and in
  exponent notation outside that range (`1e300`, `2.5e-7`); `inf`/`-inf`/`nan` as
  those words.
- `Bool` → `true`/`false`; `String` → itself (unquoted); `Unit` → `()`.
- `List` → `[1, 2, 3]`; `Map` → `{k: v, ...}` with **insertion-order** iteration;
  tuple → `(1, "two")`. **Strings nested inside containers are quoted**
  (`["a", "b"]`), with escapes for `\n`, `\t`, `\"`, `\\`.
- Struct → `Point { x: 1.0, y: 2.0 }` (declaration field order); enum →
  `Circle(1.0)` / `None` (variant name without the enum name).
- Functions → `<fn name>` / `<fn>` for lambdas.

### 8.2 `List[T]` methods

`len() -> Int`, `is_empty() -> Bool`, `push(T) -> Unit`, `pop() -> Option[T]`,
`insert(Int, T) -> Unit`, `remove(Int) -> T` (panics OOB),
`get(Int) -> Option[T]`, `first() -> Option[T]`, `last() -> Option[T]`,
`contains(T) -> Bool`, `index_of(T) -> Option[Int]`,
`reverse() -> List[T]` (returns new; there is no `reversed` alias),
`sort() -> List[T]` (returns a new sorted list; elements must be Int/Float/String —
a concrete violation is a compile error E0322, and a violation reached through a
generic type parameter panics at runtime), `sort_by(fn(T, T) -> Int) -> List[T]` (comparator returns
negative/zero/positive; both sorts are **stable** — equal elements keep their
input order, so tied results are deterministic), `map[U](fn(T) -> U) -> List[U]`,
`filter(fn(T) -> Bool) -> List[T]`, `each(fn(T) -> Unit) -> Unit`,
`fold[A](A, fn(A, T) -> A) -> A`, `any(fn(T) -> Bool) -> Bool`,
`all(fn(T) -> Bool) -> Bool`, `find(fn(T) -> Bool) -> Option[T]`,
`flat_map[U](fn(T) -> List[U]) -> List[U]`, `zip[U](List[U]) -> List[(T, U)]`
(length = shorter), `enumerate() -> List[(Int, T)]`,
`slice(Int, Int) -> List[T]` (start inclusive, end exclusive, clamped, new list),
`concat(List[T]) -> List[T]` (new list), `join(String) -> String`
(only `List[String]`), `clone() -> List[T]` (shallow), `clear() -> Unit`.

`contains`, `index_of` use structural equality. List literals: `[1, 2, 3]`, `[]`.
The callback-taking methods (`map`, `filter`, `each`, `fold`, `any`, `all`, `find`,
`flat_map`, `sort_by`) iterate a snapshot of the receiver taken at the call, so a
callback that mutates the list does not disturb the iteration.

### 8.3 `String` methods

`len() -> Int` (count of Unicode scalars), `byte_len() -> Int`,
`is_empty() -> Bool`, `chars() -> List[String]`,
`split(String) -> List[String]` (empty separator → chars; adjacent separators
produce empty strings, like Rust), `trim() -> String`, `to_upper() -> String`,
`to_lower() -> String` (ASCII-only case mapping),
`contains(String) -> Bool`, `starts_with(String) -> Bool`,
`ends_with(String) -> Bool`, `replace(String, String) -> String`,
`slice(Int, Int) -> String` (by chars, clamped),
`char_at(Int) -> Option[String]`, `code_at(Int) -> Option[Int]` (the Unicode
scalar value at a char index, v0.6; inverse of the free `char()` function),
`index_of(String) -> Option[Int]` (char index),
`index_of_from(String, Int) -> Option[Int]` (v0.6: search from a char index;
the result is still an absolute char index; out-of-range starts find nothing,
except that the empty pattern matches at the end),
`repeat(Int) -> String`, `pad_left(Int, String) -> String`,
`pad_right(Int, String) -> String`,
`trim() -> String`, `trim_start() -> String`, `trim_end() -> String`
(Unicode whitespace; v0.6 added the one-sided pair), `parse_int() -> Option[Int]`
(optional sign, decimal only, no surrounding spaces),
`parse_float() -> Option[Float]`, `parse_hex() -> Option[Int]` (v0.8: an
optional `0x`/`0X` prefix then hex digits, case-insensitive, no sign — the
inverse of `to_hex()`, so `n.to_hex().parse_hex() == Some(n)` for every
`Int`; invalid digits or an empty string give `None`),
`to_string() -> String` (identity),
`to_bytes() -> Bytes` (the UTF-8 encoding; the inverse bridge is
`Bytes.utf8()`, § 8.4b).

### 8.4 `Map[K, V]` methods

`len() -> Int`, `is_empty() -> Bool`, `get(K) -> Option[V]`,
`insert(K, V) -> Option[V]` (returns the previous value),
`remove(K) -> Option[V]`, `contains_key(K) -> Bool`,
`keys() -> List[K]`, `values() -> List[V]`, `entries() -> List[(K, V)]`,
`clear() -> Unit`, `clone() -> Map[K, V]` (shallow).

Iteration order is **insertion order** (deterministic). Map literals:
`{"a": 1, "b": 2}`; the empty map literal is `{:}` (because `{}` is an empty block).

### 8.4b `Bytes` methods (v0.7)

Constructed by the free functions `bytes(n)` (zero-filled; negative n
panics) and `bytes_of(List[Int])` (panics on values outside 0..255).

`len() -> Int`, `get(Int) -> Int` (panics OOB), `set(Int, Int) -> Unit`
(panics OOB or on non-byte values), `push(Int) -> Unit` (checked),
`push_bytes(Bytes) -> Unit` / `push_str(String) -> Unit` (bulk appends:
a whole buffer and a string's UTF-8 bytes; `push_bytes` snapshots its
argument first, so appending a buffer to itself appends its old
contents), `push_u16le(Int)` / `push_i16le(Int)` / `push_u32le(Int)`
and the big-endian pair `push_u16be(Int)` / `push_u32be(Int)`
(multi-byte appends with range checks — wire formats need no bitwise
operators), `push_u64le(Int)` / `push_u64be(Int)` (v0.8; no range check —
`Int` already *is* the 64-bit two's-complement value, so its own bit
pattern is what gets written; one method covers the u64/i64 split the
16/32-bit pushers need, since at 64 bits the two are the same operation),
`read_u16le(Int) -> Int` / `read_i16le(Int) -> Int` /
`read_u32le(Int) -> Int` / `read_u16be(Int) -> Int` /
`read_u32be(Int) -> Int` / `read_u64le(Int) -> Int` / `read_u64be(Int) ->
Int` (v0.8 for the 64-bit pair; the multi-byte reads back, at a byte
offset; a read that would touch any byte outside the buffer panics like
`get`; unlike the 16/32-bit reads, a 64-bit read can come back negative
when bit 63 is set, matching `to_hex`/hex-literal semantics),
`push_f32le(Float)` / `push_f32be(Float)` / `read_f32le(Int) -> Float` /
`read_f32be(Int) -> Float` (v0.8: `Float` is `f64`; these narrow to `f32`
at the boundary — the wire format vertex/uniform/PCM data actually uses —
so a read-back is only accurate to `f32`'s precision, not bit-identical to
whatever `f64` went in),
`slice(Int, Int) -> Bytes` (clamped copy, like `List.slice`),
`concat(Bytes) -> Bytes` (new buffer), `to_list() -> List[Int]`,
`utf8() -> Result[String, String]` (UTF-8 decode). `String.to_bytes() ->
Bytes` is the inverse bridge. Bytes may be map keys (content-hashed);
mutating a key afterward strands the entry, as with other mutable keys.

### 8.4c `Worker` methods (v0.7)

A `Worker` is the parent's handle to a spawned isolate (§ 7, the `worker`
namespace). `Worker` is a nameable type usable in signatures and
annotations (`fn spawn_pool(n: Int) -> List[Worker]`). It displays as
`<worker>`, cannot be compared with `==` or ordered, and cannot be a
map key.

`send(String) -> Bool` (`false` once the worker has finished),
`recv() -> Option[String]` (**blocks**; `None` means the worker finished
and every message it sent has been received),
`try_recv() -> Option[Option[String]]` (v0.8: the non-blocking twin —
never waits; outer `None` = no message ready right now, `Some(None)` =
`recv`'s own terminal state one level deeper, `Some(Some(s))` = a message —
lets a parent poll several workers without picking one to block on),
`join() -> Result[Unit,
String]` (**blocks**: hangs up the parent's send side — a worker blocked
in `worker.recv()` sees `None` — then waits; `Err` carries the worker's
panic message; joining again returns the cached result; messages the
worker sent before finishing can still be `recv`'d after `join`).

### 8.4d `Window` methods (v0.8, Linux + Windows + macOS (Apple Silicon))

A `Window` is the handle `window.create` returns (§ 7.3) — an OS window
plus a current GL context. `Window` is a nameable type usable in signatures
and annotations, like `Worker`. It displays as `<window>`, cannot be
compared with `==` or ordered, and cannot be a map key.

`poll() -> Unit` (pumps the platform event queue — the X event queue on
Linux, the Win32 message queue on Windows, `NSApp`'s event queue on macOS;
updates should-close/key/mouse/size state — call this once per frame),
`should_close() -> Bool` (`true` once the window manager's close button —
caught via `WM_DELETE_WINDOW` on Linux, `WM_CLOSE` on Windows, an
`isVisible` check after the close box is clicked on macOS — was clicked, or
after `close()`), `close() -> Unit` (explicit early teardown: releases the
GL context, destroys the window, and closes the display connection (Linux)
/ releases the device context (Windows) / releases the `NSOpenGLContext`
and `NSWindow` (macOS); idempotent — calling it again, or letting the
`Window` be collected afterward, is a no-op).

Unlike `Worker` (cheap, plentiful OS threads — fine to leak a collected
handle until process exit), a GL context + window is a comparatively scarce
OS/GPU resource: a `Window` that is garbage-collected without an explicit
`close()` tears down eagerly at collection time via the same teardown
`close()` runs, so a program that opens and discards many windows in a loop
actually reclaims them as it goes.

`key_down(String) -> Bool` (is the named key currently held down; names are
X11 keysym names — lowercase letters like `"w"`, `"a"`; an unrecognized name
returns `false` rather than erroring), `mouse_pos() -> (Float, Float)` (last
known pointer position within the window, updated by `poll()`; `(0.0, 0.0)`
before the first pointer motion), `width() -> Int` / `height() -> Int`
(current window size in pixels, updated by `poll()` on a resize),
`clear(r: Float, g: Float, b: Float, a: Float) -> Unit` (`glClearColor` +
`glClear(GL_COLOR_BUFFER_BIT)`), `swap_buffers() -> Unit` (presents the back
buffer — the window is double-buffered).

`make_current() -> Unit` (v0.8): makes this window's GL context current on
this thread. Idempotent, and the same call `clear()`/`swap_buffers()`
already make internally per call — this just exposes it as its own public
method, so the `gfx` namespace (§ 7.4) has an explicit window to target:
every `gfx.*` call operates against whichever window last called
`make_current()`.

`backend_name() -> String` (v0.8): `"opengl"`, `"metal"` (macOS,
`create_metal`), or `"vulkan"` (Linux, `create_vulkan`) — see the macOS
Metal and Linux Vulkan backend notes above (§ 7.3). On Windows, where only
the OpenGL backend exists, this always returns `"opengl"`.

### 8.5 Numeric methods

`Int`: `to_float() -> Float`, `to_string() -> String`, `abs() -> Int`,
`pow(Int) -> Int` (panics on negative exponent or overflow),
`min(Int) -> Int`, `max(Int) -> Int`.

Int bit intrinsics (v0.7), all over the 64-bit two's-complement
pattern: `count_ones() -> Int` (popcount), `leading_zeros() -> Int`,
`trailing_zeros() -> Int` (both zero-count methods define the 0 case as
64, matching Rust), `ushr(Int) -> Int` (**logical** — zero-filling —
right shift, the unsigned complement to the arithmetic `>>`; the count
shares `>>`'s contract and panics outside 0..=63),
`rotate_left(Int) -> Int` / `rotate_right(Int) -> Int` (the count is
taken mod 64 — Rust rotate semantics — so unlike shifts they never
panic; a negative count rotates the other way), `to_hex() -> String`
(lowercase minimal hex of the two's-complement bit pattern:
`(-1).to_hex() == "ffffffffffffffff"`, `255.to_hex() == "ff"`,
`0.to_hex() == "0"`; fixed widths via `pad_left`).

Wrapping arithmetic (v0.8): `wrapping_add(Int) -> Int` /
`wrapping_sub(Int) -> Int` / `wrapping_mul(Int) -> Int` truncate to the low
64 bits instead of panicking like the checked `+`/`-`/`*` operators —
for hash finalizers and similar bit-mixing code. One 64-bit primitive
covers a 32-bit wrap too: mask after wrapping (`a.wrapping_mul(b) &
0xFFFFFFFF`), rather than a separate 32-bit-specific intrinsic.

`Float`: `to_int() -> Int` (truncates toward zero; panics on NaN or out of Int
range), `to_string() -> String`, `abs()`, `floor()`, `ceil()`, `round()`,
`sqrt()`, `is_nan() -> Bool`, `to_fixed(Int) -> String` (v0.6: exactly n
decimal places, half-away-from-zero as in Rust's `{:.n}`; n clamps to
`[0, 17]`; a result that is all zeros drops its minus sign, so no `-0.00`).

### 8.6 `Option[T]` / `Result[T, E]` methods

`Option[T]`: `is_some()`, `is_none()`, `unwrap() -> T` (panics on `None`),
`unwrap_or(T) -> T`, `map[U](fn(T) -> U) -> Option[U]`,
`and_then[U](fn(T) -> Option[U]) -> Option[U]`, `or(Option[T]) -> Option[T]`.
`Result[T, E]`: `is_ok()`, `is_err()`, `unwrap() -> T` (panics on `Err`, showing the
error), `unwrap_or(T) -> T`, `unwrap_err() -> E` (panics on `Ok`),
`map[U](fn(T) -> U) -> Result[U, E]`, `map_err[F](fn(E) -> F) -> Result[T, F]`,
`and_then[U](fn(T) -> Result[U, E]) -> Result[U, E]`.

### 8.7 `Range` methods

`to_list() -> List[Int]`, `contains(Int) -> Bool`, `len() -> Int`,
`map[U](fn(Int) -> U) -> List[U]`, `filter(fn(Int) -> Bool) -> List[Int]`,
`each(fn(Int) -> Unit) -> Unit`, `fold[A](A, fn(A, Int) -> A) -> A`,
`rev() -> List[Int]`, `any(fn(Int) -> Bool) -> Bool` / `all(fn(Int) ->
Bool) -> Bool` (v0.8, short-circuiting like `List`'s — previously reachable
only via `.to_list().any(..)`, an unneeded allocation). Ranges are values:
`let r = 1..10;`.

### 8.8 Universal method

`.to_string()` is **not** universal; use `str(x)`. Only the types listed above have
methods; calling an unknown method is a compile error naming the receiver type.

---

## 9. Grammar (EBNF)

```ebnf
program     = { item | stmt } ;
item        = [ "pub" ] ( fn_decl | struct_decl | enum_decl | let_stmt )
            | impl_decl | import_stmt ;
impl_decl   = "impl" IDENT [ generics ] "{" { method_decl } "}" ;
method_decl = [ "pub" ] "fn" IDENT [ generics ] "(" "self" [ "," params ] ")" [ "->" type ] block ;
import_stmt = "import" IDENT { "." IDENT } [ "as" IDENT ] ";" ;
fn_decl     = "fn" IDENT [ generics ] "(" [ params ] ")" [ "->" type ] block ;
generics    = "[" IDENT { "," IDENT } "]" ;
params      = param { "," param } [ "," ] ;
param       = IDENT ":" type ;
struct_decl = "struct" IDENT [ generics ] "{" [ fields ] "}" ;
fields      = field { "," field } [ "," ] ;
field       = IDENT ":" type ;
enum_decl   = "enum" IDENT [ generics ] "{" variant { "," variant } [ "," ] "}" ;
variant     = IDENT [ "(" type { "," type } ")" ] ;

stmt        = let_stmt | assign_or_expr_stmt | while_stmt | for_stmt
            | return_stmt | break_stmt | continue_stmt ;
let_stmt    = "let" [ "mut" ] let_pattern [ ":" type ] "=" expr ";" ;
let_pattern = IDENT | tuple_pat | struct_pat ;
assign_or_expr_stmt = expr [ assign_op expr ] ";"?   (* ";" required unless final in block *)
assign_op   = "=" | "+=" | "-=" | "*=" | "/=" | "%="
            | "&=" | "|=" | "^=" | "<<=" | ">>=" ;
while_stmt  = "while" expr block | "while" "let" pattern "=" expr block ;
for_stmt    = "for" let_pattern "in" expr block ;
return_stmt = "return" [ expr ] ";" ;

type        = "Int" | "Float" | "Bool" | "String" | "Unit"
            | IDENT [ "." IDENT ] [ "[" type { "," type } "]" ]  (* named / generic / module-qualified *)
            | "(" type "," type { "," type } ")"          (* tuple *)
            | "fn" "(" [ type { "," type } ] ")" [ "->" type ]
            | "List" "[" type "]" | "Map" "[" type "," type "]" | "Range" ;

expr        = or_expr | if_expr | match_expr | block | lambda ;
lambda      = "|" [ lambda_params ] "|" ( [ "->" type ] block | expr ) ;
lambda_params = lambda_param { "," lambda_param } [ "," ] ;
lambda_param  = ( IDENT | "_" ) [ ":" type ] ;
if_expr     = "if" expr block [ "else" ( if_expr | block ) ]
            | "if" "let" pattern "=" expr block [ "else" ( if_expr | block ) ] ;
match_expr  = "match" expr "{" arm { "," arm } [ "," ] "}" ;
arm         = pattern [ "if" expr ] "->" arm_body ;
arm_body    = expr | "return" [ expr ] | "break" | "continue" ;
pattern     = or_pat ;
or_pat      = base_pat { "|" base_pat } ;
base_pat    = literal | "_" | IDENT
            | "(" pattern "," pattern { "," pattern } ")"
            | path [ "(" pattern { "," pattern } ")" ]     (* enum variant; path may be module-qualified *)
            | path "{" [ field_pats ] [ ".." ] "}" ;       (* struct; ditto *)
```

(Expression grammar follows the precedence table in § 3.1; `?` is a postfix
operator at level 9, and struct-literal names may be module-qualified.
A struct-literal field is `IDENT ":" expr` or the shorthand `IDENT` — § 2.3.)

---

## 10. Tooling behavior

- `fable run file.fable [args...]` (or `fable file.fable [args...]`) —
  compile and run; everything after the script path reaches the program as
  `os.args()`. Imports resolve file-relative, then via `FABLE_PATH`
  (colon-separated directories).
- `fable check file.fable` — compile only; print diagnostics.
- `fable dis file.fable` — print disassembled bytecode.
- `fable fmt file.fable [more.fable ...]` — print the canonically
  formatted source of every named file (`--write` to modify in place;
  flags may appear anywhere among the files). A file that fails to parse
  is reported, the remaining files still format, and the exit code is
  nonzero. Formatting is width-aware: constructs that fit within 100
  columns keep a one-line layout, longer ones break (call arguments one
  per line with a trailing comma, method chains before each `.` after
  the first, binary expressions before each operator, an `if`/`else if`
  chain either fits entirely on one line or breaks every branch, and so
  on); `--width N` overrides the limit. A single token longer than the
  width (usually a string literal) is never split. A bracketed literal
  or argument list with interior comments never collapses to one line:
  each element keeps its own line, own-line comments stay before their
  element and trailing comments stay on its line — so a comment doubles
  as an escape hatch for meaning-bearing multi-line layout (e.g. a
  hand-drawn 2-D grid). Formatting is idempotent.
- `fable test [paths...]` (v0.4) — run golden tests: every `.fable` file
  found is a test, checked against `//? expect:` / `//? error:` /
  `//? panic:` directives in its comments (a file with no directives must
  merely run silently). Exit 0 when all pass, 1 otherwise. The
  interpreter's own spec suite runs through the same code. Two precision
  rules (v0.6): a directive counts only when `//?` **begins a line
  comment** — prose in `//` or `/* */` comments and string literals (even
  nested in interpolation holes) that merely mention `//?` are inert —
  and expected/actual lines are compared ignoring trailing whitespace
  (trailing spaces in a directive are invisible in an editor, so they can't
  be pinned reliably). Unknown flags are rejected with a usage message.
  `--bless` (v0.8) re-pins: a stdout-only mismatch whose actual and expected
  line counts already agree has its `//? expect:` lines rewritten in place
  to match the actual output instead of failing (a line-count change means
  a print statement was added or removed, which directive corresponds to
  which new line is then ambiguous, so that case is left as a normal
  failure for a human to re-pin; `//? error:`/`//? panic:` directives are
  never rewritten either way). Automates the manual "run, pipe through
  `sed`, append, re-run" workflow (demos/STYLE.md § 1) for the common case
  of a value changing without the print statements around it changing.
- `fable lsp` (v0.4) — a language server over stdio: diagnostics on
  open/change (identical to `fable check`, imports included), hover with
  checked types, go-to-definition across module files, and completion
  (v0.5) for methods, fields, module members, namespaces, and top-level
  names — answered from the last good analysis, so it works mid-edit.
- `fable build <dir|file.fable> [-o OUT] [--launcher PATH]` (v0.7) — pack a
  program into one self-contained executable. Every file under the program's
  directory is stapled onto a copy of the interpreter — appended after its
  image as `payload ‖ u64(payload_len) ‖ "FABLZOO1"`, a dependency-free
  little-endian archive — and the program is type-checked first, so a build
  never ships a binary that fails to compile. On startup such a binary reads
  its own 16-byte trailer, and if the magic is present unpacks the payload
  into a per-process scratch directory, makes it the working directory, and
  runs the entry (`main.fable`); an ordinary `fable` has no trailer and is
  unaffected. Files are packed under the path *as given*, so a stapled binary
  behaves exactly like `fable <that path>` run from the build directory —
  imports, `fs.*`, `worker.spawn`, and output are all identical. `--launcher`
  supplies interpreter bytes cross-compiled for another target (stapling is
  host-independent, so one machine can assemble binaries for every target);
  `-o` sets the output path (default: the program directory's name).
  `--payload-only` writes just the archive (no launcher) — the macOS build
  links it in as a `__DATA,__fablezoo` Mach-O section instead of appending,
  since a Mach-O with data past `__LINKEDIT` cannot be code-signed.
- `fable repl` — interactive REPL; expressions print their value (in `str` form,
  except `String` values print quoted) unless the value is `()`. Imports
  work (v0.5) and persist across inputs.
- `fable tokens file.fable` / `fable ast file.fable` — debugging dumps.

Diagnostics use the format:

```
error[E0301]: type mismatch
  --> examples/foo.fable:3:9
   |
 3 |     let x: Int = "hi";
   |            ---   ^^^^ expected `Int`, found `String`
   |            expected due to this
```

Error code ranges: E01xx lexing, E02xx parsing, E03xx types, E04xx name resolution,
E05xx pattern matching, E06xx other semantic errors. Warnings: W01xx.

## 11. Implementation limits

Programs exceeding these limits get a clean diagnostic (never silent
misbehavior): 255 parameters per function/lambda, 255 fields per enum variant,
60,000 fields per struct, 60,000 elements per list/map/tuple literal or string
interpolation, 60,000 local variables per function, 65,000 global bindings,
2,000 levels of syntactic nesting (E0207), 20,000 nested operations per
expression (E0324). At runtime: 4,096 call frames ("stack overflow" panic) —
the cap is configurable via the `FABLE_MAX_DEPTH` environment variable
(v0.6; floor 64; malformed values warn and keep the default), for recursive
tree-walking workloads whose depth is data-dependent. One honest caveat:
recursion that passes *through native callbacks* (`map`/`sort_by`
comparators, `try`) also consumes the interpreter's own native stack, so a
very large cap can turn the graceful, catchable panic into a hard process
abort in such programs — raise it generously but not astronomically.
Display nesting deeper than 10,000 levels renders as `...`,
and equality on values whose *map* nesting exceeds 64 levels panics.

One behavioral caveat: map keys are hashed at insertion. **Mutating a list,
map, or struct after using it as a key strands the entry** — it still counts
toward `len()` and appears in `keys()`, but no lookup can reach it. Don't
mutate values used as keys (most languages ban mutable keys outright; Fable
trusts you instead).
