# The Fable Book

A guided tour of the Fable programming language. Every code snippet in these
chapters was executed against the real interpreter before it was written
down; output shown is real output.

1. [Getting Started](01-getting-started.md) — building, running, the CLI,
   the REPL, and how to read a Fable error message.
2. [Fundamentals](02-fundamentals.md) — values, bindings, operators, strings
   and interpolation, control flow, and how programs execute.
3. [Functions and Closures](03-functions-and-closures.md) — declarations,
   lambdas, capture semantics, higher-order functions, and generics.
4. [Structs, Enums, and Pattern Matching](04-data.md) — modeling data, and
   the exhaustiveness checking that keeps `match` honest.
5. [Collections and Strings](05-collections-and-strings.md) — Lists, Maps,
   tuples, Ranges, and the string toolbox.
6. [Under the Hood](06-under-the-hood.md) — bytecode, closures at runtime,
   the garbage collector, and what `fable dis` shows you.

For the terse normative rules, see the [language specification](../docs/SPEC.md);
for implementation internals, [ARCHITECTURE.md](../docs/ARCHITECTURE.md).
