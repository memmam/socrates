# The Fable Language Specification

**Version 0.1** — This document is the normative reference for the Fable programming
language. The implementation (`src/`), the golden test suite (`tests/spec/`), and the
book (`book/`) must all agree with this document.

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
  Out-of-range literals are a compile error (E0105).
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
+ - * / %  == != < <= > >=  && || !  =  -> =>  . , : ; ( ) [ ] { }  .. ..=  |  _  ?
```

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
| 6     | `+` `-`                  | left  | Int, Float; `+` also String ++ String |
| 7     | `*` `/` `%`              | left  | Int, Float (`%` Int only) |
| 8     | unary `-` `!`            | —     | Int/Float; Bool |
| 9     | call `f(x)`, index `a[i]`, field `.x`, method `.m(x)`, tuple index `.0`, try `?` | left | |

Comparison operators are **non-associative**: `a < b < c` is a parse error.
Integer division truncates toward zero; `/` or `%` by integer zero **panics**.
Float division by zero yields `inf`/`nan` per IEEE-754.

The arithmetic operators (and unary `-`) also apply to user types through
**operator methods** (v0.3, § 5.1): `a + b` dispatches to `a.add(b)` when
`a`'s type defines one. `==`/`!=` remain structural for all types and cannot
be overloaded; compound assignment (`+=`) never dispatches.

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
- `while cond { ... }` and `for x in iterable { ... }` are statements (they cannot
  appear where a value is required). `for` iterates over a `List[T]` (yielding `T`),
  a `Range` (yielding `Int`), or a `String` (yielding one-character `String`s, by
  Unicode scalar). The loop variable is a fresh immutable binding each iteration —
  closures created in the body capture that iteration's value. A `for` loop over a
  list iterates the **live** list by index (elements pushed during the loop are
  visited; removed elements are skipped), whereas the callback methods (`each`,
  `map`, ...) iterate a **snapshot** taken when the method is called.
- `break` and `continue` are only valid inside loops. `return expr` / `return`
  exits the enclosing function (top-level `return` is a compile error).

### 3.4 Lambdas

```fable
let double = |x: Int| x * 2
let add = |a, b| a + b           // OK if the context determines the types
nums.map(|n| n * n)
let f = |x: Int| -> Int { x + 1 }   // full form with return type and block
```

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
- All arms must have the same type; `match` is an expression.

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
is a compile error).

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
- Compound assignment: `x += e`, `-=`, `*=`, `/=`, `%=` (sugar; same rules as `=`).
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
- The REPL and one-shot string evaluation cannot import (E0334).

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
| `os.args()` | `fn() -> List[String]` | CLI args after the script path |
| `os.env(name)` | `fn(String) -> Option[String]` | |
| `os.run(cmd, args)` | `fn(String, List[String]) -> Result[(Int, String, String), String]` | `Ok((exit code, stdout, stderr))`; `Err` if the binary can't launch |
| `os.exit(code)` | `fn(Int) -> Unit` | ends the process immediately |
| `os.time()` | `fn() -> Float` | Unix-epoch seconds (`clock()` is monotonic) |

Namespaced (no import needed): `math.pi`, `math.e`, `math.sqrt(Float)`,
`math.sin/cos/tan/atan/atan2/log/log2/exp` (Float), `math.pow(Float, Float)`,
`math.floor/ceil/round(Float) -> Float`, `math.abs_int(Int) -> Int`,
`math.abs(Float) -> Float`, `math.min/max(Int, Int) -> Int`,
`math.min_float/max_float(Float, Float) -> Float`, `math.random() -> Float`
(uniform [0, 1), xorshift PRNG), `math.seed(Int) -> Unit`.

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
`reverse() -> List[T]` (returns new), `reversed` alias — **no**, only `reverse`,
`sort() -> List[T]` (returns a new sorted list; elements must be Int/Float/String —
a concrete violation is a compile error E0322, and a violation reached through a
generic type parameter panics at runtime), `sort_by(fn(T, T) -> Int) -> List[T]` (comparator returns
negative/zero/positive), `map[U](fn(T) -> U) -> List[U]`,
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
`char_at(Int) -> Option[String]`, `index_of(String) -> Option[Int]` (char index),
`repeat(Int) -> String`, `pad_left(Int, String) -> String`,
`pad_right(Int, String) -> String`, `parse_int() -> Option[Int]`
(optional sign, decimal only, no surrounding spaces),
`parse_float() -> Option[Float]`, `to_string() -> String` (identity).

### 8.4 `Map[K, V]` methods

`len() -> Int`, `is_empty() -> Bool`, `get(K) -> Option[V]`,
`insert(K, V) -> Option[V]` (returns the previous value),
`remove(K) -> Option[V]`, `contains_key(K) -> Bool`,
`keys() -> List[K]`, `values() -> List[V]`, `entries() -> List[(K, V)]`,
`clear() -> Unit`, `clone() -> Map[K, V]` (shallow).

Iteration order is **insertion order** (deterministic). Map literals:
`{"a": 1, "b": 2}`; the empty map literal is `{:}` (because `{}` is an empty block).

### 8.5 Numeric methods

`Int`: `to_float() -> Float`, `to_string() -> String`, `abs() -> Int`,
`pow(Int) -> Int` (panics on negative exponent or overflow),
`min(Int) -> Int`, `max(Int) -> Int`.
`Float`: `to_int() -> Int` (truncates toward zero; panics on NaN or out of Int
range), `to_string() -> String`, `abs()`, `floor()`, `ceil()`, `round()`,
`sqrt()`, `is_nan() -> Bool`.

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
`rev() -> List[Int]`. Ranges are values: `let r = 1..10;`.

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
assign_op   = "=" | "+=" | "-=" | "*=" | "/=" | "%=" ;
while_stmt  = "while" expr block ;
for_stmt    = "for" IDENT "in" expr block ;
return_stmt = "return" [ expr ] ";" ;

type        = "Int" | "Float" | "Bool" | "String" | "Unit"
            | IDENT [ "." IDENT ] [ "[" type { "," type } "]" ]  (* named / generic / module-qualified *)
            | "(" type "," type { "," type } ")"          (* tuple *)
            | "fn" "(" [ type { "," type } ] ")" [ "->" type ]
            | "List" "[" type "]" | "Map" "[" type "," type "]" | "Range" ;

expr        = or_expr | if_expr | match_expr | block | lambda ;
lambda      = "|" [ lambda_params ] "|" ( [ "->" type ] block | expr ) ;
if_expr     = "if" expr block [ "else" ( if_expr | block ) ] ;
match_expr  = "match" expr "{" arm { "," arm } [ "," ] "}" ;
arm         = pattern [ "if" expr ] "->" expr ;
pattern     = or_pat ;
or_pat      = base_pat { "|" base_pat } ;
base_pat    = literal | "_" | IDENT
            | "(" pattern "," pattern { "," pattern } ")"
            | path [ "(" pattern { "," pattern } ")" ]     (* enum variant; path may be module-qualified *)
            | path "{" [ field_pats ] [ ".." ] "}" ;       (* struct; ditto *)
```

(Expression grammar follows the precedence table in § 3.1; `?` is a postfix
operator at level 9, and struct-literal names may be module-qualified.)

---

## 10. Tooling behavior

- `fable run file.fable [args...]` (or `fable file.fable [args...]`) —
  compile and run; everything after the script path reaches the program as
  `os.args()`. Imports resolve file-relative, then via `FABLE_PATH`
  (colon-separated directories).
- `fable check file.fable` — compile only; print diagnostics.
- `fable dis file.fable` — print disassembled bytecode.
- `fable fmt file.fable` — print the canonically formatted source
  (`--write` to modify in place). Formatting is idempotent.
- `fable repl` — interactive REPL; expressions print their value (in `str` form,
  except `String` values print quoted) unless the value is `()`.
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
expression (E0324). At runtime: 4,096 call frames ("stack overflow" panic),
display nesting deeper than 10,000 levels renders as `...`, and equality on
values whose *map* nesting exceeds 64 levels panics.

One behavioral caveat: map keys are hashed at insertion. **Mutating a list,
map, or struct after using it as a key strands the entry** — it still counts
toward `len()` and appears in `keys()`, but no lookup can reach it. Don't
mutate values used as keys (most languages ban mutable keys outright; Fable
trusts you instead).
