# Structs, Enums, and Methods

This chapter is about modeling data: structs for "this *and* that", enums for
"this *or* that", and `match` for taking data apart again — with the compiler
checking that you handled every case. Every snippet is a complete program you
can save and run with `socrates run file.soc`.

## Structs

A `struct` declares a nominal record type. Construct one with a literal that
names every field, read fields with `.`, and assign to them with `=`:

```soc
struct Point { x: Float, y: Float }

let p = Point { x: 1.0, y: 2.0 };
println(p.x);
p.y = 5.0;
println(p);

let x = 9.0;
let y = 9.0;
println(Point { x, y });     // shorthand: bare `x` means `x: x`
```

```text
1.0
Point { x: 1.0, y: 5.0 }
Point { x: 9.0, y: 9.0 }
```

Struct instances are mutable heap objects: `p.y = 5.0` works even though `p`
is a plain `let`, because the *binding* still points at the same object —
`mut` governs rebinding, not field mutation. Fields may be given in any
order, but all must be present — no partial construction, no defaults.
`Point { x: 1.0 }` is rejected with `error[E0427]: missing field `y` in
struct literal`; unknown fields are likewise compile errors.

## Structs are references

Assigning a struct to another binding does not copy it. Both names refer to
the same object, and mutation through one is visible through the other:

```soc
struct Counter { n: Int }

let a = Counter { n: 0 };
let b = a;            // b and a point at the same object
b.n += 1;
b.n += 1;
println(a.n);
println(a == b);
```

```text
2
true
```

`==` is structural (it compares field values, not identity), so two
independently built `Counter { n: 2 }` values also compare equal. There is
no built-in `clone` for structs — to copy one, build a new literal
from the old fields. Tuples, by contrast, are immutable values; a struct is
the right choice when you want named fields or shared, mutable state.

## Generic structs

Structs take type parameters in square brackets, and the arguments are
inferred from the literal — there is no turbofish:

```soc
struct Pair[A, B] { first: A, second: B }

let p = Pair { first: 1, second: "one" };     // Pair[Int, String] inferred
let q = Pair { first: true, second: 0.5 };    // Pair[Bool, Float]
println(p);
println(q.second);
```

```text
Pair { first: 1, second: "one" }
0.5
```

(Strings nested inside a printed container are quoted.) Operations on your
types can be free functions — a handful of types followed by the functions
that work on them is a fine shape for a Socrates program — or methods in an
`impl` block (see below). There are still no traits.

## Enums

An `enum` is a choice among *variants*, each optionally carrying a payload.
Variants are written `EnumName.Variant`; nullary variants take no
parentheses. The natural way to consume an enum is `match`:

```soc
enum Shape {
    Circle(Float),
    Rect(Float, Float),
    Empty,
}

fn describe(s: Shape) -> String {
    match s {
        Shape.Circle(r) -> "a circle of radius {r}",
        Shape.Rect(w, h) -> "a {w} by {h} rectangle",
        Shape.Empty -> "nothing at all",
    }
}

println(describe(Shape.Circle(2.0)));
println(describe(Shape.Rect(3.0, 4.0)));
println(describe(Shape.Empty));
println(Shape.Circle(2.0));
```

```text
a circle of radius 2.0
a 3.0 by 4.0 rectangle
nothing at all
Circle(2.0)
```

`match` is an expression: each arm is `pattern -> expression`, arms are
checked to have the same type, and the whole thing produces a value. Unlike
structs, enum values are immutable — to "change" one, build a new one.
Constructing a variant requires the enum name (`Circle(1.0)` alone is
`error[E0400]: undefined name `Circle``); the one exception is next.

## Option and Result are ordinary enums

Socrates has no null. The prelude defines two enums you will use constantly:
`enum Option[T] { Some(T), None }` and `enum Result[T, E] { Ok(T), Err(E) }`.
Only two things about them are special: the `?` operator (chapter 6)
propagates their failure cases, and — a courtesy — their variants may be
used *unqualified*, both when constructing and in patterns. `Some(5)` and `Option.Some(5)` are
the same value.

```soc
fn safe_div(a: Int, b: Int) -> Result[Int, String] {
    if b == 0 { Err("cannot divide {a} by zero") } else { Ok(a / b) }
}

fn show(r: Result[Int, String]) -> String {
    match r {
        Ok(v) -> "got {v}",
        Err(msg) -> "failed: {msg}",
    }
}

println(show(safe_div(10, 3)));
println(show(safe_div(10, 0)));
```

```text
got 3
failed: cannot divide 10 by zero
```

Both types also carry combinator methods (`unwrap_or`, `map`, `and_then`,
and friends) for when a full `match` is more ceremony than you need; the
error-handling chapter covers them.

## The pattern language

Patterns are where `match` earns its keep. Literals match themselves,
or-patterns (`|`) try alternatives, guards (`if`) add arbitrary conditions,
and a lowercase name *binds* whatever reaches it:

```soc
fn describe(n: Int) -> String {
    match n {
        0 -> "zero",
        1 | 2 | 3 -> "a few",
        n if n < 0 -> "negative",
        n -> "many ({n})",
    }
}

println(describe(0));
println(describe(2));
println(describe(-4));
println(describe(100));
```

```text
zero
a few
negative
many (100)
```

Arms are tried top to bottom; the first pattern that matches (and whose
guard, if any, is `true`) wins. `_` is a binding that doesn't bother with a
name. Or-patterns work on variants too (`Key.Up | Key.Down -> "arrow"`), and
all alternatives must bind the same names with the same types.

Tuples destructure positionally, and patterns nest — here literals sit
inside tuple patterns:

```soc
fn locate(point: (Int, Int)) -> String {
    match point {
        (0, 0) -> "at the origin",
        (x, 0) -> "on the x axis at {x}",
        (0, y) -> "on the y axis at {y}",
        (x, y) -> "out at ({x}, {y})",
    }
}

println(locate((0, 0)));
println(locate((3, 0)));
println(locate((0, -2)));
println(locate((5, 7)));
```

```text
at the origin
on the x axis at 3
on the y axis at -2
out at (5, 7)
```

Struct patterns match on fields: `field: pattern` matches a field against a
nested pattern, bare `field` is shorthand for `field: field`, and `..`
ignores the fields you don't name (omitting fields *without* `..` is an
error):

```soc
struct Pixel { x: Int, y: Int, brightness: Float }

fn classify(p: Pixel) -> String {
    match p {
        Pixel { x: 0, y: 0, .. } -> "corner pixel",
        Pixel { brightness: b, .. } if b > 0.9 -> "hot pixel",
        Pixel { x, y, .. } -> "pixel at ({x}, {y})",
    }
}

println(classify(Pixel { x: 0, y: 0, brightness: 0.5 }));
println(classify(Pixel { x: 4, y: 2, brightness: 0.95 }));
println(classify(Pixel { x: 4, y: 2, brightness: 0.1 }));
```

```text
corner pixel
hot pixel
pixel at (4, 2)
```

## `if let` and `while let`: matching one pattern

Testing a single pattern doesn't need a whole `match`. `if let PATTERN =
EXPR { ... }` runs its block only when `EXPR` matches, with `else` for
everything else:

```soc
fn describe(o: Option[Int]) -> String {
    if let Some(n) = o {
        "got {n}"
    } else {
        "nothing"
    }
}

println(describe(Some(42)));
println(describe(None));
```

```text
got 42
nothing
```

`while let` drains something one item at a time, stopping the moment the
pattern stops matching — no `is_empty()` check, no hand-rolled `while true
{ match ... { _ -> break } }`:

```soc
import std.deque;

let q = deque.from_list([1, 2, 3]);
let mut total = 0;
while let Some(x) = q.pop_front() {
    total += x;
}
println(total);
```

```text
6
```

Both are sugar for `match`: `if let PAT = E { T } else { F }` is exactly
`match E { PAT -> T, _ -> F }`, and `while let PAT = E { B }` is exactly
`while true { match E { PAT -> B, _ -> break } }`. Because they desugar to
an ordinary `match`, they inherit its rules for free — with no `else`, the
arm must be `Unit`, exactly like a plain `if` with no `else`.

## Exhaustiveness: the compiler keeps score

A `match` must cover every possible value of its scrutinee. If it doesn't,
that's a compile error — and Socrates reports a concrete value you missed, not
just "add more arms":

```soc errors
enum Shape { Circle(Float), Rect(Float, Float), Empty }

fn area(s: Shape) -> Float {
    match s {
        Shape.Circle(r) -> math.pi * r * r,
    }
}
```

```text
error[E0501]: non-exhaustive match: the value `Shape.Rect(_, _)` is not covered
  --> demo.soc:4:11
   |
4 |     match s {
   |           ^ `Shape.Rect(_, _)` is not covered
  note: add an arm for it, or a catch-all `_ ->` arm
```

This is the payoff of enums: add a variant to `Shape` next month and every
`match` that forgot about it stops compiling, with a witness pointing at the
gap. The checker is deliberately conservative about guards — a guard might
be `false`, so a guarded arm never counts toward coverage:

```soc errors
fn sign(n: Int) -> String {
    match n {
        0 -> "zero",
        n if n > 0 -> "positive",
    }
}
```

```text
error[E0501]: non-exhaustive match: the value `-1` is not covered
  --> demo.soc:2:11
   |
2 |     match n {
   |           ^ `-1` is not covered
  note: add an arm for it, or a catch-all `_ ->` arm
```

Relatedly, `Int`, `Float`, and `String` literal patterns can never make a
match exhaustive on their own (there are too many values), so those matches
always need a binding or `_` arm. `Bool` is the exception: `true` and
`false` together are exhaustive.

The opposite mistake — an arm that earlier arms already cover — is a
warning, not an error. The program still compiles and runs:

```soc
enum Coin { Heads, Tails }

fn name(c: Coin) -> String {
    match c {
        Coin.Heads -> "heads",
        Coin.Tails -> "tails",
        _ -> "impossible",
    }
}

println(name(Coin.Heads));
```

```text
warning[W0101]: unreachable match arm
  --> demo.soc:7:9
   |
7 |         _ -> "impossible",
   |         ^ this pattern is covered by earlier arms
heads
```

This is also why a reflexive catch-all arm is an anti-pattern in Socrates: that
`_` is dead weight today, and tomorrow it will silently absorb the
`Coin.Edge` variant you add, instead of triggering an exhaustiveness error.

## Destructuring `let`

Patterns are not confined to `match`. A `let` can destructure, as long as
the pattern is *irrefutable* — guaranteed to match. Tuple and struct
patterns qualify:

```soc
struct Point { x: Float, y: Float }

let pair = (3, "three");
let (num, word) = pair;
println("{num} is spelled {word}");

let p = Point { x: 1.5, y: 2.5 };
let Point { x, y } = p;
println(x + y);
```

```text
3 is spelled three
4.0
```

A variant pattern can fail, so it is rejected in `let` at compile time:

```soc errors
let opt = Some(5);
let Some(n) = opt;
println(n);
```

```text
error[E0503]: refutable pattern in a `let` binding
  --> demo.soc:2:5
   |
2 | let Some(n) = opt;
   |     ^^^^^^^ this variant pattern can fail
  note: the pattern must always match here; use `match` instead
```

## Early exit from a match arm: `return`

An arm body may be a bare `return` (or `break`/`continue` inside a loop) —
sugar for the one-statement block `{ return ...; }`. An arm
that exits never produces a value, so the checker exempts it from the "all
arms have the same type" rule. That makes `match` a tidy way to peel off the
failure case and continue with the success value:

```soc
fn summarize(xs: List[Int]) -> String {
    let first = match xs.first() {
        None -> return "empty list",
        Some(n) -> n,
    };
    let total = xs.fold(0, |acc, x| acc + x);
    "starts at {first}, sums to {total}"
}

println(summarize([3, 4, 5]));
println(summarize([]));
```

```text
starts at 3, sums to 12
empty list
```

Here `first` is an `Int` — the `None` arm never yields a value because it
exits `summarize` entirely. This idiom is the workhorse of Socrates error
handling when the failure case needs its own logic; when it would just be
`return`, the `?` operator (chapter 6) says the same thing in one character.
`examples/json.soc` uses both. (Assignment stays statement-only: an arm
that assigns still needs a block body, `Some(v) -> { x = v; }`, and the
error message says so.)

## Recursive data: a binary search tree

Enum variants can carry values of the enum's own type, which is all you need
for trees and lists. Here is a binary search tree — `insert` returns a new
tree sharing untouched subtrees with the old one, and `to_list` reads the
values back in sorted order:

```soc
enum Tree {
    Leaf,
    Node(Tree, Int, Tree),
}

fn insert(t: Tree, v: Int) -> Tree {
    match t {
        Tree.Leaf -> Tree.Node(Tree.Leaf, v, Tree.Leaf),
        Tree.Node(l, x, r) ->
            if v < x { Tree.Node(insert(l, v), x, r) }
            else if v > x { Tree.Node(l, x, insert(r, v)) }
            else { t },
    }
}

fn to_list(t: Tree) -> List[Int] {
    match t {
        Tree.Leaf -> [],
        Tree.Node(l, x, r) -> to_list(l).concat([x]).concat(to_list(r)),
    }
}

let mut tree = Tree.Leaf;
for v in [5, 3, 8, 1, 4, 8] {
    tree = insert(tree, v);
}
println(to_list(tree));
```

```text
[1, 3, 4, 5, 8]
```

Note the duplicate `8` is inserted once — the `else { t }` arm returns the
existing tree unchanged. Patterns nest to any depth, so structural questions
read almost like diagrams: `Tree.Node(Tree.Leaf, _, Tree.Leaf)` matches
exactly the nodes whose children are both leaves. The same trick builds
linked lists (`enum IntList { Nil, Cons(Int, IntList) }`) or, with a struct
plus `Option`, mutable nodes: `struct Node { value: Int, next: Option[Node] }`.
In practice, reach for the built-in `List[T]` first.

## Methods: `impl` blocks

Builtin types have methods — `xs.map(f)`, `s.trim()`. Your own types get
them from an `impl` block. The first parameter of a method is a bare `self`
(no annotation — it always has the impl type); everything else is an
ordinary typed parameter:

```soc
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

```text
5.0
10.0
```

Methods are just functions whose first argument is the receiver: `p.len()`
compiles to the same call as `len(p)` would. They are hoisted like
functions, so order within a file does not matter, and they pair naturally
with `match` on enums:

```soc
enum Shape { Circle(Float), Rect(Float, Float), Empty }

impl Shape {
    fn area(self) -> Float {
        match self {
            Shape.Circle(r) -> math.pi * r * r,
            Shape.Rect(w, h) -> w * h,
            Shape.Empty -> 0.0,
        }
    }
}

println(Shape.Rect(3.0, 4.0).area());
```

```text
12.0
```

Generic types re-bind their type parameters in the `impl` header, and a
method may add its own after the type's:

```soc
struct Pair[A, B] { first: A, second: B }

impl Pair[A, B] {
    fn swap(self) -> Pair[B, A] {
        Pair { first: self.second, second: self.first }
    }
}

println(Pair { first: 1, second: "one" }.swap().first);   // one
```

```text
one
```

A few rules: multiple `impl` blocks per type are fine, but a method name may
be defined only once per type. Impl targets must be your own structs or
enums — `impl Int` or `impl Option` is a compile error, since the builtins
keep their curated method sets.

## Operator methods

The well-known method names `add`, `sub`, `mul`, `div`, `rem`, and `neg`
overload the operators themselves. Define `add` on a type and `a + b`
dispatches to `a.add(b)`:

```soc
struct V2 { x: Float, y: Float }

impl V2 {
    fn add(self, o: V2) -> V2 { V2 { x: self.x + o.x, y: self.y + o.y } }
    fn mul(self, k: Float) -> V2 { V2 { x: self.x * k, y: self.y * k } }
    fn neg(self) -> V2 { V2 { x: -self.x, y: -self.y } }
}

let a = V2 { x: 1.0, y: 2.0 };
let b = V2 { x: 10.0, y: 20.0 };
let c = a + b * 2.0;            // ordinary precedence: mul before add
println((-c).x);
```

```text
-21.0
```

Dispatch is on the *left* operand's type, so mixed signatures like
`vec * scalar` are natural — the parameter and return types are whatever the
method declares. Two deliberate refusals keep the feature honest: `==` stays
structural for every type and cannot be overloaded, and `+=` never
dispatches (write `x = x + y`).

## Where we are

You can now define your own types — structs (named fields, mutable,
reference semantics) and enums (immutable tagged choices), both generic —
give them methods and operators, and take them apart with patterns checked
for exhaustiveness and reachability. The failure cases you modeled with
`Option` and `Result` get their own toolkit next: combinators, the `?`
operator, and catchable panics.
