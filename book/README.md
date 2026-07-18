# The Socrates Book

A guided tour of the Socrates programming language, organized by topic. Every
code snippet in these chapters is executed against the real interpreter in
CI before it is published; the output shown is real output.

1. [Getting Started](01-getting-started.md) — building, running, the CLI,
   the REPL, and how to read a Socrates error message.
2. [Fundamentals](02-fundamentals.md) — values, bindings, operators
   (including bitwise), strings and interpolation, control flow, and how
   programs execute.
3. [Functions and Closures](03-functions-and-closures.md) — declarations,
   lambdas, capture semantics, higher-order functions, generics, and tail
   calls.
4. [Structs, Enums, and Methods](04-data.md) — modeling data, the
   exhaustiveness checking that keeps `match` honest, `impl` blocks, and
   operator overloading.
5. [Collections, Strings, and Bytes](05-collections-and-strings.md) — Lists,
   Maps, tuples, Ranges, the string toolbox, and raw binary buffers.
6. [Error Handling](06-error-handling.md) — `Option` and `Result`,
   combinators, the `?` operator, `try`, and what panics.
7. [Programs Across Files](07-modules.md) — modules and `import`, `pub`
   visibility, module semantics, and the `SOCRATES_PATH` search path.
8. [The Standard Library and System Namespaces](08-stdlib.md) — `fs`/`os`,
   the embedded standard library (json, collections, iterators, `std.glm`
   3D math), and native graphics with `window` and `gfx`.
9. [Workers, `fft`, and the GPU](09-workers.md) — parallel isolates, the
   native FFT namespace, and feature-gated GPU compute.
10. [Under the Hood](10-under-the-hood.md) — bytecode, closures at runtime,
    the garbage collector, and what `socrates dis` shows you.
11. [The Toolchain](11-toolchain.md) — `socrates test`, the formatter, the
    `socrates lsp` language server, and the disassembler.
12. [Idioms and Style](12-idioms.md) — writing Socrates well: the fast, clear,
    parallelizable habits, and the design values behind them.

For the terse normative rules, see the [language specification](../docs/SPEC.md);
for implementation internals, [ARCHITECTURE.md](../docs/ARCHITECTURE.md).
