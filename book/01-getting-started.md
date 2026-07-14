# Getting Started

Fable ships as a single Rust crate with zero dependencies. This chapter gets
you from a fresh checkout to a running program, walks through every CLI
subcommand, spends some quality time in the REPL, and teaches you to read a
Fable error message — a skill you will use more than any other.

## Building

You need a Rust toolchain and nothing else:

```sh
cargo build              # debug build → target/debug/fable
cargo build --release    # optimized build → target/release/fable
```

The rest of this chapter writes `fable` for whichever binary you built. If
you want it on your `PATH`, copy or symlink `target/release/fable` somewhere
convenient — there is no installer, and the binary is self-contained.

Running `fable` with no arguments (or with anything it doesn't understand)
prints the usage summary:

```text
The Fable programming language

USAGE:
    fable <file.fable>            compile and run
    fable run <file.fable>        compile and run
    fable check <file.fable>      type-check only
    fable dis <file.fable>        show compiled bytecode
    fable fmt <file.fable> [-w]   format source (print, or -w to rewrite)
    fable tokens <file.fable>     dump tokens (debug)
    fable ast <file.fable>        dump the AST (debug)
    fable repl                    interactive session

ENVIRONMENT:
    FABLE_GC_STRESS=1    collect garbage before every allocation
    FABLE_GC_LOG=1       log collections to stderr
```

## Hello, Fable

A Fable program is a single file, executed top to bottom. There is no `main`
function. Put this in `hello.fable`:

```fable
println("Hello, Fable!");
```

Run it — `run` is the default subcommand, so both of these work:

```sh
fable hello.fable
fable run hello.fable
```

```text
Hello, Fable!
```

Something slightly more real, to prove types and string interpolation are
along for the ride:

```fable
fn greet(name: String) -> String {
    "Hello, {name}!"
}

let friends = ["Ada", "Grace", "Aesop"];
for name in friends {
    println(greet(name));
}
println("{friends.len()} greetings sent.");
```

```text
Hello, Ada!
Hello, Grace!
Hello, Aesop!
3 greetings sent.
```

A few things to notice, all covered properly in later chapters: functions
declare their parameter and return types, but `friends` gets its type
(`List[String]`) by inference; `{name}` inside a string is interpolation,
not a format directive; and the last expression of a function body — here
the interpolated string — is its return value, no `return` needed.

## The other subcommands

**`fable check`** compiles without running. Use it as a fast "does this even
typecheck" loop, or in CI:

```sh
$ fable check greet.fable
ok: no errors
```

Errors, if any, print in the diagnostic format described below, and the exit
code tells scripts what happened.

**`fable fmt`** prints the canonically formatted source; add `-w` (or
`--write`) *after the filename* to rewrite the file in place. Given this
crime scene:

```fable
fn   square( n:Int )->Int { n*n }
let mut total=0;
for i in 1..=5 { total+=square( i ); }
println("total = {total}");
```

`fable fmt messy.fable` prints:

```fable
fn square(n: Int) -> Int {
    n * n
}
let mut total = 0;
for i in 1..=5 {
    total += square(i);
}
println("total = {total}");
```

Formatting is idempotent — running `fmt` on its own output changes nothing —
and it preserves comments.

**`fable dis`** shows the bytecode the compiler produced, one function at a
time. For a file containing `square` and a call to it:

```fable
fn square(n: Int) -> Int {
    n * n
}

println(square(7));
```

```text
; constants
;   [0] 7

fn square (proto 0, arity 1, 0 upvalues, max locals 3)
     0  get_local   0
     1  get_local   0
     2  mul
     3  return

fn <script> (proto 1, arity 0, 0 upvalues, max locals 1)
     0  const       0 ; 7
     1  call_fn     0 argc=1 ; square
     2  call_native println argc=1
     3  pop
     4  unit
     5  return
```

Top-level code compiles into an implicit `<script>` function. You never need
`dis` to write Fable, but it is the best window into what the VM chapter
talks about.

**`fable tokens`** and **`fable ast`** are debugging dumps of the two
earliest compiler stages. Tokens are compact:

```sh
$ fable tokens hello.fable
1:1     Ident("println")
1:8     LParen
1:9     Str("Hello, Fable!")
1:24    RParen
1:25    Semi
2:1     Eof
```

`fable ast` prints the parsed tree in Rust's debug format, with spans and
node IDs — verbose enough that even `hello.fable` produces a screenful.
Reach for these only when you suspect the compiler is misreading your
source, or when you're hacking on Fable itself.

## The REPL

`fable repl` starts an interactive session. You type Fable code; expressions
print their value *and its type*:

```text
Fable 0.1.0 — type a program, or :help
fable> 1 + 2 * 3
7 : Int
fable> let double = |x: Int| x * 2;
fable> [1, 2, 3].map(double)
[2, 4, 6] : List[Int]
fable> "hello".to_upper()
"HELLO" : String
```

Statements like `let` print nothing. Unit values are suppressed — calling
`println` shows the printed line but no `() : Unit` noise. String *results*
print quoted (`"HELLO"`) so you can tell them apart from output your program
printed itself.

`:type` tells you an expression's type without evaluating it:

```text
fable> :type double
: fn(Int) -> Int
```

Input continues across lines while a delimiter is open — braces, brackets,
parentheses — with a continuation prompt:

```text
fable> fn fact(n: Int) -> Int {
  ...>     if n < 2 { 1 } else { n * fact(n - 1) }
  ...> }
fable> fact(10)
3628800 : Int
```

Mistakes don't end the session. A definition that fails to compile is rolled
back, and everything defined before it survives:

```text
fable> let x: Int = "hi";
error[E0301]: type mismatch
  --> <repl-1>:1:14
   |
1 | let x: Int = "hi";
   |        ---   ^^^^ expected `Int`, found `String`
   |        expected due to this
fable> let x = 42;
fable> x
42 : Int
```

The full command list is short: `:help`, `:type <expr>`, and `:q` to quit.

## Reading a diagnostic

Fable's compile errors follow one format, so it pays to dissect one
specimen thoroughly. This program mixes an `Int` and a `Float`, which Fable
never converts implicitly:

```fable errors
let price = 3;
let tax = 0.2;
println(price + tax);
```

```text
error[E0320]: mismatched operand types `Int` and `Float`
  --> mixnum.fable:3:15
   |
3 | println(price + tax);
   |         ----- ^ --- this is `Float`
   |         this is `Int`
  note: Fable has no implicit numeric conversion; use `.to_float()` or `.to_int()`
```

Anatomy, top to bottom:

- **The header** — `error[E0320]: mismatched operand types ...` — gives the
  severity, a stable error code, and a one-line summary. Codes are grouped
  by compiler stage: `E01xx` lexing, `E02xx` parsing, `E03xx` types,
  `E04xx` name resolution, `E05xx` pattern matching, `E06xx` other semantic
  checks. Warnings use `W01xx`.
- **The location line** — `--> mixnum.fable:3:15` — file, line, column of
  the primary span.
- **The labels** — the quoted source line with underlines beneath it. The
  `^` underline marks the primary span (here the `+` operator, which is
  where the two types collide); `-` underlines mark secondary spans, each
  with its own message. One diagnostic can label several spans — here it
  points at both operands and names each one's type.
- **The note** — advice that applies to the error as a whole rather than to
  one span. This one tells you the fix: `price.to_float() + tax` or
  `price + tax.to_int()`, your choice.

Not every error carries every part. Some have a single label and no note;
some, like an undefined-name error, add a did-you-mean note:

```fable errors
let count = 3;
println(count + conut);
```

```text
error[E0400]: undefined name `conut`
  --> typo.fable:2:17
   |
2 | println(count + conut);
   |                 ^^^^^ not found in this scope
  note: did you mean `count`?
```

## Exit codes

The `fable` binary reports what happened through its exit code, which
matters the moment you put it in a script or a Makefile:

| Code | Meaning |
|------|---------|
| `0`  | success |
| `64` | bad command line (unknown subcommand, no arguments) |
| `65` | the program failed to compile (any error diagnostic) |
| `66` | the input file could not be read |
| `70` | the program compiled but **panicked** at runtime |

A panic is a runtime abort — index out of bounds, integer overflow,
`unwrap()` on `None`, a failed `assert`, and friends. It prints a message
and a stack trace to stderr:

```fable panics
let xs = [1, 2, 3];
println(xs[7]);
```

```text
panic: list index out of bounds: index 7, length 3
  at <script> (panic.fable:2:9)
```

with exit code `70`, whereas the type errors above exit with `65`.

## The garbage collector's environment variables

Fable's runtime uses a tracing mark-and-sweep collector. Two environment
variables expose it:

**`FABLE_GC_LOG=1`** logs every collection to stderr — handy for seeing the
heap breathe. This program builds a 20,000-element list of strings:

```fable
let mut rows = [];
for i in 0..20000 {
    rows.push("row {i}");
}
println(rows.len());
```

```text
$ FABLE_GC_LOG=1 fable gcdemo.fable
[gc] collected 126 of 256 objects (130 live, next at 260)
[gc] collected 65 of 260 objects (195 live, next at 390)
[gc] collected 98 of 390 objects (292 live, next at 584)
...
[gc] collected 5614 of 22458 objects (16844 live, next at 33688)
20000
```

(Output elided in the middle; you'll see a dozen or so lines as the heap
threshold doubles its way up.)

**`FABLE_GC_STRESS=1`** forces a collection before *every* allocation. It
exists to flush out rooting bugs in the runtime itself — if a program works
normally but breaks under stress mode, that's a Fable bug, and the whole
test suite runs under it. It makes allocation-heavy programs dramatically
slower; there is no reason to use it day to day, but it's reassuring that
you can.

## Where to next

You have a working toolchain and can decode its complaints. You have also
seen hints of the language: inference, interpolation, lambdas,
expression-bodied functions. The next chapters tour the language proper —
values and types, control flow, pattern matching, and the collection
library.

One expectation to set now: Fable is deliberately small. v0.2 added methods
on your own types, multi-file modules, the `?` operator, and tail-call
optimization (chapter 7 covers all four) — but there are still no traits,
no visibility modifiers, and no package manager. What the language does
include, it checks thoroughly at compile time, as the diagnostics above
suggest.
