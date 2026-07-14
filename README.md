# The Fable Programming Language

Fable is a statically-typed, garbage-collected programming language with
algebraic data types, exhaustive pattern matching, closures, and generics —
implemented from scratch in Rust with **zero dependencies**.

```fable
enum Shape {
    Circle(Float),
    Rect(Float, Float),
}

fn area(s: Shape) -> Float {
    match s {
        Shape.Circle(r) -> math.pi * r * r,
        Shape.Rect(w, h) -> w * h,
    }
}

let shapes = [Shape.Circle(1.0), Shape.Rect(3.0, 4.0)];
let total = shapes.map(|s| area(s)).fold(0.0, |a, x| a + x);
println("total area: {total}");
```

Everything here — lexer, parser, unification-based type inference,
Maranget exhaustiveness checking, bytecode compiler, stack VM, mark-and-sweep
garbage collector, REPL, formatter, and disassembler — lives in about 13,700
lines of dependency-free Rust in `src/`, exercised by 200+ golden spec tests
(every one a runnable Fable program) and a book whose every snippet was
executed against the interpreter before it was written down.

## Highlights

- **Real type inference.** `let xs = [1, 2, 3];` — types flow through
  generics, lambdas, and collections. `xs.map(|n| n * 2)` needs no
  annotations; genuinely ambiguous programs get a targeted error instead of a
  guess.
- **Pattern matching that's checked.** A `match` missing a case is a compile
  error *with a concrete witness*: `the value Shape.Rect(_, _) is not covered`.
  Unreachable arms warn. Or-patterns, guards, nested destructuring, and struct
  patterns all participate.
- **Rust-quality diagnostics.** Errors carry stable codes, underlined
  multi-span labels, and did-you-mean suggestions:

  ```text
  error[E0301]: type mismatch
    --> demo.fable:3:18
     |
   3 |     let x: Int = "hi";
     |            ---   ^^^^ expected `Int`, found `String`
     |            expected due to this
  ```

- **A real GC.** Tracing mark-and-sweep with a checkpoint-based rooting
  discipline. Run any program with `FABLE_GC_STRESS=1` to collect before
  *every* allocation — the whole test suite passes under it.
- **Closures done properly.** Lua-style upvalues: captured variables are
  shared by reference, live past their scope, and close automatically —
  two closures over one `let mut` counter see each other's increments.
- **String interpolation.** `"sum = {a + b}"` with arbitrary nested
  expressions — including nested strings with their own interpolations.
- **Batteries.** ~110 built-in methods across `List`, `Map`, `String`,
  `Option`, `Result`, `Range`, `Int`, `Float`, plus a `math` namespace.
- **A whole toolchain**: `run`, `check`, a REPL with persistent incremental
  compilation and `:type`, a comment-preserving formatter (`fmt`), and a
  bytecode disassembler (`dis`).

## Try it

```sh
cargo build --release

# Run a program
./target/release/fable examples/mandelbrot.fable

# A raytracer written in Fable (writes a PPM image)
./target/release/fable examples/raytracer.fable > scene.ppm

# An interpreter running inside an interpreter
./target/release/fable examples/brainfuck.fable

# Poke at the machinery
./target/release/fable dis examples/algorithms.fable
./target/release/fable repl
```

```text
fable> let double = |x: Int| x * 2;
fable> [1, 2, 3].map(double)
[2, 4, 6] : List[Int]
fable> :type |acc: Int, x: Int| acc + x
: fn(Int, Int) -> Int
```

## A four-minute tour

```fable
// Bindings are immutable unless marked `mut`; types are inferred.
let name = "Aesop";
let mut count = 0;

// Functions declare parameter types; everything else is inferred.
fn fib(n: Int) -> Int {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

// Generics use explicit brackets and infer at call sites.
fn largest[T](xs: List[T], better: fn(T, T) -> Bool) -> Option[T] {
    xs.fold(None, |best, x| match best {
        None -> Some(x),
        Some(b) -> if better(x, b) { Some(x) } else { Some(b) },
    })
}
println(largest([3, 1, 4, 1, 5], |a, b| a > b));   // Some(5)

// Structs are nominal records with reference semantics.
struct Point { x: Float, y: Float }
let p = Point { x: 1.0, y: 2.0 };
p.x += 10.0;

// Enums + match: exhaustiveness is enforced at compile time.
enum Tree {
    Leaf(Int),
    Node(Tree, Tree),
}
fn sum(t: Tree) -> Int {
    match t {
        Tree.Leaf(v) -> v,
        Tree.Node(l, r) -> sum(l) + sum(r),
    }
}

// Option and Result are built in, with combinators.
let n = "42".parse_int().map(|v| v * 2).unwrap_or(0);

// Collections know functional and imperative tricks alike.
let squares = (1..=10).map(|n| n * n).filter(|n| n % 2 == 0);
let index: Map[String, Int] = {:};
index["one"] = 1;

// Loops, ranges, and destructuring.
for pair in squares.enumerate() {
    let (i, sq) = pair;
    println("{i}: {sq}");
}
```

More in the [book](book/) — from a guided tour through the standard library
reference to a chapter on how the VM works.

## Project layout

```
src/
  lexer.rs        tokens, nested string interpolation, comments
  parser.rs       recursive descent → AST (error-recovering)
  check.rs        inference, generics, name resolution, mutability
  patterns.rs     exhaustiveness/reachability (Maranget usefulness)
  compiler.rs     AST → bytecode (closures, match compilation)
  vm.rs           the stack machine + GC checkpoint rooting
  value.rs        heap objects, mark-and-sweep collector
  natives.rs      the built-in function/method implementations
  builtins.rs     their type schemes (shared with the checker)
  fmt.rs          comment-preserving formatter
  repl.rs         incremental REPL with rollback
  dis.rs          disassembler
docs/SPEC.md      the normative language specification
book/             the Fable book
tests/spec/       golden tests (expect / error / panic directives)
examples/         mandelbrot, raytracer, game of life, brainfuck,
                  JSON parser, algorithms, a tiny text adventure
```

## Testing

```sh
cargo test                      # unit tests + the golden spec suite
FABLE_GC_STRESS=1 cargo test    # same, collecting before every allocation
```

Golden tests are plain Fable programs with expectations in comments:

```fable
println(1 + 2 * 3);   //? expect: 7
let x: Int = "no";    //? error: type mismatch
[1, 2][9];            //? panic: out of bounds
```

## Status

Fable is a complete, working language built as a demonstration project.
The spec (`docs/SPEC.md`) is the source of truth; deviations are bugs.
Things deliberately out of scope for v0.1: user-defined methods/traits,
multi-file modules, a `?` operator, and tail-call optimization.

## License

MIT.
