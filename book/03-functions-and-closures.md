# Functions and Closures

Functions in Socrates are ordinary values: you can bind them, pass them, return
them, and stash them in lists. Closures capture their surroundings by
reference — properly, with upvalues that outlive their scope. This chapter
covers named functions, lambdas, capture semantics, and generic functions.

## Declaring functions

A named function declares its parameter types; the return type comes after
`->`. The body is a block, and the block's tail expression is the return
value:

```soc
fn greet(name: String) -> String {
    "Hello, {name}!"
}

fn area(w: Float, h: Float) -> Float {
    w * h
}

println(greet("Socrates"));
println(area(3.0, 4.5));
```

```text
Hello, Socrates!
13.5
```

Parameter annotations are not optional. Inference works everywhere else, but
a named function's signature is its contract, and Socrates insists you write it
down — `fn double(x) -> Int` is a parse error: `` error[E0200]: expected `:`
(parameter types are required), found `)` ``.

## The return type defaults to `Unit`

Omit `-> Type` and the function returns `Unit` — fine for functions called
only for their effects. The default bites when you *meant* to return
something: a body whose tail expression has a non-`Unit` type won't typecheck
against the implicit `Unit`, and the compiler guesses both likely causes:

```soc errors
fn double(x: Int) {
    x * 2
}
println(double(4));
```

```text
error[E0301]: function `double` should return `Unit`, but its body has type `Int`
  --> demo.soc:2:5
   |
2 |     x * 2
   |     ^^^^^ this has type `Int`
  note: without a `-> Type` annotation, functions return `Unit`; did you forget the annotation, or a trailing `;`?
```

## Early exit with `return`

The tail expression is the idiomatic way to produce a value, but `return
expr;` exits immediately from anywhere in the body — handy for guard clauses:

```soc
fn classify(n: Int) -> String {
    if n == 0 {
        return "zero";
    }
    if n % 2 == 0 { "even" } else { "odd" }
}

println(classify(0));
println(classify(3));
println(classify(8));
```

```text
zero
odd
even
```

In a `Unit` function, a bare `return;` exits early the same way. `return`
only makes sense inside a function — at the top level of a script it is a
compile error: ``error[E0304]: `return` outside of a function``.

## Hoisting and mutual recursion

Function declarations are hoisted: a function is visible everywhere in the
file, including above its own declaration, so top-level code may call a
function defined further down. It also means mutually recursive functions
need no forward declarations — define them in whatever order reads best:

```soc
fn is_even(n: Int) -> Bool {
    if n == 0 { true } else { is_odd(n - 1) }
}

fn is_odd(n: Int) -> Bool {
    if n == 0 { false } else { is_even(n - 1) }
}

println(is_even(10));
println(is_odd(10));
```

```text
true
false
```

One restriction: `fn` declarations live at the top level only. There are no
nested named functions — the compiler points you at the alternative:

```soc errors
fn outer() -> Int {
    fn helper() -> Int { 1 }
    helper() + 1
}
println(outer());
```

```text
error[E0201]: function declarations are only allowed at the top level
  --> demo.soc:2:5
   |
2 |     fn helper() -> Int { 1 }
   |     ^^^^^^^^^^^^^^^^^^^^^^^^ declared here
  note: use a lambda instead: `let f = |x| ...;`
```

## Lambdas

A lambda is an anonymous function expression: parameters between pipes, then
a body. The body is a single expression, or — with an optional `-> Type` —
a block. Zero parameters is `||`:

```soc
let double = |x: Int| x * 2;
let add = |a: Int, b: Int| a + b;
let hello = || println("hello from a lambda");
let clamp = |x: Int| -> Int {
    if x < 0 { return 0; }
    if x > 100 { return 100; }
    x
};

println(double(21));
println(add(2, 3));
hello();
println(clamp(-5));
println(clamp(700));
```

```text
42
5
hello from a lambda
0
100
```

(`return` inside a lambda exits the lambda, not the enclosing function.)

Unlike named functions, lambda parameters may omit their types when the
context supplies them — as an argument to `map`, `filter`, or `fold`, or
against an annotated binding:

```soc
let nums = [1, 2, 3, 4, 5];
println(nums.map(|n| n * n));
println(nums.filter(|n| n % 2 == 1));
println(nums.fold(0, |acc, n| acc + n));

let apply: fn(Int) -> Int = |x| x + 1;
println(apply(41));
```

```text
[1, 4, 9, 16, 25]
[1, 3, 5]
15
42
```

The body alone can be enough, too: `x + 1` only works at one type, so
`let inc = |x| x + 1;` infers `fn(Int) -> Int` with no context at all.

## Why `|x| x` fails to infer

The identity lambda has nothing to constrain its parameter — not the body
(`x` works at any type) and, below, not the context either:

```soc errors
let id = |x| x;
```

```text
error[E0302]: cannot infer the type of this lambda parameter
  --> demo.soc:1:11
   |
1 | let id = |x| x;
   |           ^ add a type annotation
  note: Socrates's inference is local; annotate this or its context
```

The deeper reason: a lambda gets exactly **one** type. Socrates's inference is
local unification with no let-generalization — polymorphism comes only from
explicit `[T]` parameter lists on named functions (below). A later use *can*
pin the lambda down: `let id = |x| x; println(id(5));` compiles, with `id:
fn(Int) -> Int`. But then that is the lambda's only type — adding
`println(id("five"));` afterwards is a type mismatch (``expected `Int`, found
`String` ``). If you need identity at many types, write a generic function:
`fn identity[T](x: T) -> T { x }`.

## Functions are values

A named function used without parentheses is a value of function type. Bind
it, pass it wherever a `fn(...) -> ...` is expected, or print it:

```soc
fn square(x: Int) -> Int { x * x }

let f = square;
println(f(9));
println([1, 2, 3].map(square));

println(square);
println(|x: Int| x);
```

```text
81
[1, 4, 9]
<fn square>
<fn>
```

Builtins are function values too. `println` itself has the generic type
`[T] fn(T) -> Unit`, so it slots straight into `each`:

```soc
[10, 20, 30].each(println);
```

```text
10
20
30
```

## Closures capture by reference

A lambda that mentions an outer variable *captures* it — by reference, not by
copy. Every closure over a variable, and the enclosing scope itself, share
one box:

```soc
let mut count = 0;

let bump = || { count += 2; };
let report = || println("count is {count}");

bump();
bump();
report();
count += 1;
report();
```

```text
count is 4
count is 5
```

`bump`'s increments are visible to `report` and to the top level; the top
level's `count += 1` is visible to `report`. One variable, three views.

## Closures outlive their scope

Captured variables survive as long as some closure still holds them — even
after the scope that declared them has returned. Each call to `make_counter`
creates a fresh `n`, owned by the closure it returns:

```soc
fn make_counter() -> fn() -> Int {
    let mut n = 0;
    || {
        n += 1;
        n
    }
}

let tick = make_counter();
let tock = make_counter();
println(tick());
println(tick());
println(tick());
println(tock());
```

```text
1
2
3
1
```

`tick` and `tock` count independently — separate calls, separate `n`s. A
closure with private mutable state is the lightest kind of object Socrates
offers — for named fields and methods, pair a struct with an `impl` block
(chapters 4 and 7).

## Generic functions

Named functions take type parameters in square brackets. Type arguments are
always inferred at the call site; there is no turbofish:

```soc
fn identity[T](x: T) -> T { x }

println(identity(5));
println(identity("five"));
```

```text
5
five
```

Type parameters can appear anywhere in the signature, including inside
function types — which is how higher-order helpers are written. Multiple type
parameters work the way you'd hope:

```soc
fn twice[T](f: fn(T) -> T, x: T) -> T {
    f(f(x))
}

println(twice(|n| n + 10, 1));
println(twice(|s| s + "!", "wow"));

fn my_map[T, U](xs: List[T], f: fn(T) -> U) -> List[U] {
    let out = [];
    for x in xs {
        out.push(f(x));
    }
    out
}

println(my_map([1, 2, 3], |n| n * n));
println(my_map([1, 2, 3], |n| "<{n}>"));
```

```text
21
wow!!
[1, 4, 9]
["<1>", "<2>", "<3>"]
```

Note the lambdas passed to `twice` and `my_map` need no annotations — the
signature is the context. Generics are erased at runtime: one compiled body
serves every instantiation.

Generic functions can return closures, too. Function composition captures
both arguments:

```soc
fn compose[A, B, C](f: fn(A) -> B, g: fn(B) -> C) -> fn(A) -> C {
    |x| g(f(x))
}

let excited = compose(|s: String| s.to_upper(), |s: String| s + "!");
println(excited("onward"));

let parse_or_zero = compose(|s: String| s.parse_int(), |o: Option[Int]| o.unwrap_or(0));
println(parse_or_zero("42"));
println(parse_or_zero("not a number"));
```

```text
ONWARD!
42
0
```

(Here the lambdas *do* need annotations: at the point `compose` is called,
nothing else determines `A`, `B`, `C`.)

## Recursion has a depth limit

Recursion is the natural way to write many functions in Socrates, but the VM's
call stack is finite: 4096 frames in the current implementation. Blow past it
and the program panics:

```soc panics
fn sum_to(n: Int) -> Int {
    if n == 0 { 0 } else { n + sum_to(n - 1) }
}

println(sum_to(1_000_000));
```

```text
panic: stack overflow
  at sum_to (demo.soc:2:32)
  at sum_to (demo.soc:2:32)
  at sum_to (demo.soc:2:32)
  ...
  at ... and 4032 more frames
```

(The runtime prints 64 frames before summarizing the rest; the middle is
elided here.) Like any panic, this exits with code 70.

The limit only applies to calls with work left pending — here every frame
holds an unfinished `+`. A call in *tail position* reuses the current frame
instead of pushing a new one, so the accumulator version runs at any depth
in constant stack space:

```soc
fn sum_acc(n: Int, acc: Int) -> Int {
    if n == 0 { acc } else { sum_acc(n - 1, acc + n) }
}

println(sum_acc(1_000_000, 0));   // one frame, a million calls
```

```text
500000500000
```

Tail position is where a call's result immediately becomes its caller's
result: the operand of `return`, the last expression of a function or lambda
body, and the result position of an `if`, `match`, or block that is itself
in tail position. Mutual recursion qualifies — two functions calling each
other in tail position bounce in a single frame — and so do calls made
through a function value.

What does *not* qualify is anything with work left to do after the call:

```soc panics
fn sum_to(n: Int) -> Int {
    if n == 0 { 0 } else { n + sum_to(n - 1) }   // the `+` runs after the call
}
println(sum_to(1_000_000));
```

```text
panic: stack overflow
```

That is the honest outcome — each frame really is holding a pending `+`, so
each frame really must exist. Rewrite with an accumulator and the optimizer
takes it from there. Ordinary bounded recursion — parsers, tree walks,
divide-and-conquer — was never in danger either way; tail calls are what let
a recursive *loop* (an interpreter's eval, a state machine) run forever in
constant space. The `dis` disassembler (chapter 10) shows a reused frame as
`tail_callfn` where an ordinary call is `call_fn`.

## Where we are

Functions round out the core language: declared at the top level with
explicit signatures, hoisted for mutual recursion, and first-class everywhere
else — lambdas that infer from context, closures that share and outlive their
scopes, and generics that keep higher-order code typed without call-site
annotations. Next up: the compound types — lists, maps, tuples, structs — and
the pattern matching that makes enums shine.
