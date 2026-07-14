# Fundamentals

This chapter covers the bones of Fable: values, bindings, operators, strings,
and control flow. Everything here is runnable — save any snippet to a file
and run it with `fable run file.fable` (or just `fable file.fable`).

## Values and types

Fable has five primitive types: `Int` (64-bit signed), `Float` (IEEE-754
double), `Bool`, `String` (immutable UTF-8), and `Unit` (the single value
`()`). Types are inferred from initializers, so annotations are rarely needed:

```fable
let answer = 42;        // Int
let ratio = 2.5;        // Float
let ready = true;       // Bool
let name = "Aesop";     // String
let nothing = ();       // Unit

println("{answer} {ratio} {ready} {name} {nothing}");
```

```text
42 2.5 true Aesop ()
```

You can annotate when you want to be explicit — `let x: Int = 5;` — and the
annotation is checked against the initializer. Integer literals may use
underscores (`1_000_000`), hex (`0x2A`), or binary (`0b1010`). Float literals
need a digit on both sides of the dot: `0.5` and `1.0` are valid, `.5` and
`1.` are not.

## No implicit conversions

Fable never converts numbers behind your back. Mixing `Int` and `Float` is a
compile error:

```fable errors
println(1 + 2.0);
```

```text
error[E0320]: mismatched operand types `Int` and `Float`
  --> demo.fable:1:11
   |
1 | println(1 + 2.0);
   |         - ^ --- this is `Float`
   |         this is `Int`
  note: Fable has no implicit numeric conversion; use `.to_float()` or `.to_int()`
```

The fix is to say what you mean. `Int` has `.to_float()`, and `Float` has
`.to_int()`, which truncates toward zero (and panics on NaN or out-of-range
values):

```fable
let n = 3;
let x = n.to_float() + 2.0;
println(x);
println(2.9.to_int());
println((-2.9).to_int());
```

```text
5.0
2
-2
```

A `Float` always displays with a decimal point or exponent (`5.0`, not `5`),
so you can tell the types apart in output.

## `let` and `let mut`

Bindings are immutable by default. To reassign, declare with `let mut`:

```fable
let mut count = 0;
count = count + 1;
count += 10;
println(count);
```

```text
11
```

The compound assignments `+=`, `-=`, `*=`, `/=`, `%=` are sugar for the
spelled-out form. Assigning to a plain `let` is a compile error:

```fable errors
let limit = 100;
limit = 200;
```

```text
error[E0307]: cannot assign to immutable binding `limit`
  --> demo.fable:2:1
   |
1 | let limit = 100;
   |     ----- declared without `mut` here
2 | limit = 200;
   | ^^^^^ cannot assign
  note: declare it as `let mut limit = ...`
```

## Shadowing

A new `let` with an old name is not assignment — it creates a fresh binding
that *shadows* the previous one, possibly with a different type. This makes
shadowing handy for step-by-step transformations:

```fable
let input = "42";
let input = input.parse_int().unwrap();
let input = input * 2;
println(input);
```

```text
84
```

The difference between assignment and shadowing shows up with scopes.
Assignment (`=`) updates the existing binding, wherever it lives; a `let`
inside a block creates a new binding that disappears when the block ends:

```fable
let mut x = 10;
if true {
    x = 20;        // assignment: updates the existing binding
}
println(x);

let y = 10;
if true {
    let y = 99;    // new binding: shadows y until the block ends
    println(y);
}
println(y);
```

```text
20
99
10
```

## Operators and precedence

The operators are the usual suspects with the usual precedence: `*`, `/`, `%`
bind tighter than `+` and `-`, which bind tighter than comparisons, which
bind tighter than `&&` (short-circuit), which binds tighter than `||`:

```fable
println(1 + 2 * 3);
println((1 + 2) * 3);
println(7 / 2);
println(-7 / 2);
println(7 % 3);
println(1 < 2 && 2 < 3);
println(!(1 == 2));
```

```text
7
9
3
-3
1
true
true
```

Integer division truncates toward zero, as `-7 / 2` shows. Equality (`==`,
`!=`) is structural and requires both sides to have the same type.
Comparisons do not chain: `1 < 2 < 3` is a parse error (`error[E0200]:
comparison operators cannot be chained; use parentheses`) — write
`1 < 2 && 2 < 3`.

`+` also concatenates strings (`"fab" + "le"` is `"fable"`), and the
ordering operators compare strings lexicographically (`"apple" < "banana"`
is `true`).

Float division by zero follows IEEE-754: `1.0 / 0.0` is `inf`. Integer
division or modulo by zero is a *panic* — a runtime error that aborts the
program with a message, a stack trace, and exit code 70 (`panic: division by
zero`). Integer arithmetic that overflows 64 bits also panics (`panic:
integer overflow`) rather than silently wrapping.

## Strings: escapes and interpolation

Any expression can be spliced into a string with `{ }`. A literal `{` is
written `\{`; a `}` outside an interpolation is just a character. The other
escapes are `\n`, `\t`, `\r`, `\\`, `\"`, `\0`, and `\u{...}` for a Unicode
scalar value:

```fable
let a = 6;
let b = 7;
println("{a} * {b} = {a * b}");
println("literal braces: \{a}");
println("tab:\tthen a \"quoted\" word");
println("caf\u{E9} costs \u{20AC}3");
```

```text
6 * 7 = 42
literal braces: {a}
tab:	then a "quoted" word
café costs €3
```

The holes hold full expressions — including `if` expressions and even other
strings with their own interpolations. The lexer tracks nesting, so inner
quotes do not end the outer string:

```fable
let hour = 23;
println("status: {if hour < 12 { "morning" } else { "evening" }}");

let name = "world";
println("outer {"inner {name}"} outer again");
```

```text
status: evening
outer inner world outer again
```

Taste is another matter — deep nesting is legal, not encouraged.

## Comments

```fable
// A line comment runs to the end of the line.
/* Block comments /* nest properly */ so you can
   comment out code that already contains comments. */
println("still here");
```

## Blocks are expressions

Fable is expression-oriented: a block evaluates to its final expression, if
that expression has no trailing semicolon. This is called the *tail
expression*:

```fable
let hypotenuse = {
    let a = 3.0;
    let b = 4.0;
    math.sqrt(a * a + b * b)
};
println(hypotenuse);
```

```text
5.0
```

The `a` and `b` here are local to the block; only the result escapes. A block
that ends with a semicolon-terminated statement has type `Unit`.

## `if`/`else` is an expression

There is no ternary operator because `if` already is one:

```fable
let score = 87;
let grade = if score >= 90 { "A" } else if score >= 80 { "B" } else { "C" };
println(grade);
```

```text
B
```

Both branches must have the same type. An `if` *without* an `else` has type
`Unit` — there would be no value when the condition is false — so using one
for its value is a compile error:

```fable errors
let x = if true { 1 };
```

```text
error[E0316]: `if` without `else` must have type `Unit`, found `Int`
  --> demo.fable:1:17
   |
1 | let x = if true { 1 };
   |                 ^^^^^ help: add an `else` branch or a `;`
```

Conditions are `Bool`, full stop — there is no truthiness, and `if n { ... }`
with an `Int` is a type error.

## Loops: `while`, `for`, and ranges

`while` repeats as long as a `Bool` condition holds:

```fable
let mut n = 1;
while n < 100 {
    n *= 2;
}
println(n);
```

```text
128
```

`for` iterates. `a..b` is a half-open range (excludes `b`); `a..=b` is
inclusive:

```fable
for i in 0..4 {
    print("{i} ");
}
println("");

for i in 1..=3 {
    println("lap {i}");
}
```

```text
0 1 2 3 
lap 1
lap 2
lap 3
```

`for` also iterates over a `List[T]`, and over a `String` by Unicode scalar —
each step yields a one-character string, so `for c in "héllo"` sees five
characters (`h`, `é`, `l`, `l`, `o`), not six bytes.

`break` exits the innermost loop; `continue` skips to its next iteration:

```fable
let mut total = 0;
for i in 1..=10 {
    if i % 2 == 0 { continue; }
    if i > 7 { break; }
    total += i;
}
println(total);
```

```text
16
```

(That is 1 + 3 + 5 + 7.) Loops are expressions of type `Unit`, and `break`
takes no value — to compute something in a loop, assign to a `mut` variable,
as above. Ranges are also first-class values with methods like `.map` and
`.to_list`; the collections chapter covers them.

## Statements, expressions, and semicolons

The semicolon rules follow from "blocks are expressions":

- Statements end with `;`.
- The final expression of a block *omits* the `;` to become the block's value.
- Declarations and block-shaped statements — `fn`, `struct`, `enum`, and
  `if`/`match`/`while`/`for` used as statements — need no trailing `;`.

This matters most in function bodies, where the tail expression is the return
value:

```fable
fn double(x: Int) -> Int {
    x * 2
}
println(double(21));
```

```text
42
```

Add a semicolon after `x * 2` and the block's value becomes `()` — and the
compiler tells you exactly that:

```fable errors
fn double(x: Int) -> Int {
    x * 2;
}
println(double(21));
```

```text
error[E0301]: function `double` should return `Int`, but its body has type `Unit`
  --> demo.fable:2:5
   |
1 | fn double(x: Int) -> Int {
   |                      --- return type declared here
2 |     x * 2;
   |     ^^^^^^ this has type `Unit`
```

`return expr;` also works, for exiting a function early; the tail expression
is just the idiomatic way to produce the final value.

## Top-level programs: order matters

A Fable program is a single file executed top to bottom. `fn`, `struct`, and
`enum` declarations are *hoisted* — visible everywhere, in any order, so
mutually recursive functions just work. But top-level `let` bindings and
statements run in order, and top-level code may only refer to globals
declared above it:

```fable errors
println(greeting);
let greeting = "hello";
```

```text
error[E0412]: `greeting` is used before its `let` declaration
  --> demo.fable:1:9
   |
1 | println(greeting);
   |         ^^^^^^^^ used here
  note: top-level code runs in order; move the `let` above this line
```

Function *bodies* are the exception: since functions are hoisted, their
bodies may mention any global, even one declared further down the file. This
is fine as long as the function isn't *called* until the global's `let` has
run. Call it too early, though, and the compiler can't save you — the
global's initializer simply hasn't run yet, and you get a panic at runtime:

```fable panics
fn shout() -> String {
    greeting.to_upper()
}

println(shout());
let greeting = "hello";
```

```text
panic: global `greeting` used before initialization
  at shout (demo.fable:2:5)
  at <script> (demo.fable:5:9)
```

Swap the last two lines — `let greeting` above `println(shout())` — and the
program prints `HELLO`. This is the one place where Fable's static checking
hands off to a runtime check, so keep your `let`s near the top of the file,
above the first top-level call that needs them. Globals follow the same mutability rules as
locals — a function may assign to a global declared `let mut` — but prefer
parameters and return values where you can.

## Where we are

You can now read and write straight-line Fable: typed values with no silent
conversions, immutable-by-default bindings, expression-based control flow,
and a strict top-to-bottom execution model. Next up: functions and closures,
then modeling data with structs, enums, and pattern matching.
