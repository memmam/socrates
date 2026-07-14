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
- **Methods on your own types** (v0.2). `impl Point { fn len(self) -> Float
  { ... } }` — generic impls, multiple blocks, dot-syntax dispatch.
- **The `?` operator** (v0.2). `Some(a.parse_int()? + b.parse_int()?)` —
  unwrap or propagate, for both `Option` and `Result`.
- **Multi-file modules** (v0.2). `import geo;` loads `geo.fable` relative to
  the importing file: qualified calls, types, variants, and patterns, with
  diamond dedup and cycle detection.
- **Tail-call optimization** (v0.2). Calls in tail position reuse the frame:
  tail recursion — direct, mutual, or through closures — runs in constant
  stack space.
- **Visibility** (v0.3). Module items are private by default; `pub` exports
  functions, types, bindings, and individual methods.
- **Operator overloading** (v0.3). `a + b` dispatches to `a.add(b)` on user
  types — Lua-metamethod style, left-operand dispatch, mixed signatures
  like `vec * 2.0` welcome. Equality stays structural.
- **A real glue language** (v0.3). `fs.read/write/list_dir/...` and
  `os.args/env/run/exit/time`, all Result-based and `?`-friendly, plus a
  `FABLE_PATH` search path for your shared modules. `examples/loc.fable` is
  a working line-counting tool in ~100 lines.
- **A standard library** (v0.4). `import std.json;` — json, flags, path,
  strings, and lazy iterators, written in Fable and embedded in the binary.
- **A test runner** (v0.4). `fable test dir/` — any `.fable` file with
  `//? expect/error/panic` directives is a test; the interpreter's own
  247-test suite uses the same command's code.
- **A language server** (v0.4). `fable lsp` — diagnostics as you type,
  hover types, go-to-definition across modules. JSON-RPC hand-rolled;
  still zero dependencies.
- **Catchable panics** (v0.4). `try(f)` turns a runtime panic into
  `Err(message)` with the VM stack fully restored — even a stack overflow.
- **A whole toolchain**: `run`, `check`, `test`, `lsp`, a REPL with
  persistent incremental compilation and `:type`, a comment-preserving
  formatter (`fmt`), and a bytecode disassembler (`dis`).

## Try it

```sh
cargo build --release

# Run a program
./target/release/fable examples/mandelbrot.fable

# A raytracer written in Fable (writes a PPM image)
./target/release/fable examples/raytracer.fable > scene.ppm

# An interpreter running inside an interpreter
./target/release/fable examples/brainfuck.fable

# A glue script: count lines of code in this repository
./target/release/fable examples/loc.fable .

# Run the golden spec suite with the built-in test runner
./target/release/fable test tests/spec

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

// Methods live in impl blocks; `self` is the receiver.
impl Tree {
    fn sum(self) -> Int {
        match self {
            Tree.Leaf(v) -> v,
            Tree.Node(l, r) -> l.sum() + r.sum(),
        }
    }
}

// Option and Result are built in, with combinators and the `?` operator.
let n = "42".parse_int().map(|v| v * 2).unwrap_or(0);
fn add_parsed(a: String, b: String) -> Option[Int] {
    Some(a.parse_int()? + b.parse_int()?)
}

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
  modules.rs      the import loader (dedup, cycles, FABLE_PATH, std)
  testing.rs      the golden-test runner (fable test + the spec suite)
  lsp.rs          the language server (diagnostics, hover, definition)
  jsonlite.rs     hand-rolled JSON for JSON-RPC
  stdlib.rs       embeds std/*.fable into the binary
  dis.rs          disassembler
std/              the standard library, written in Fable
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

v0.2 delivered everything v0.1 had declared out of scope — user-defined
methods (`impl` blocks), multi-file modules (`import`), the `?` operator,
and tail-call optimization. v0.3 made it a real glue language: `pub`
visibility, operator methods, a `FABLE_PATH` module search path, and
`fs`/`os` builtins. v0.4 built the toolchain: `fable test`, the embedded
standard library (including lazy iterators written in Fable itself),
`fable lsp`, and catchable panics. Still deliberately out of scope: full
traits (operator methods cover the common case), per-field visibility, and
a package manager.

## License

MIT.
