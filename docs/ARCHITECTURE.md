# Fable internals

This document walks the pipeline from source text to executed bytecode and
explains the load-bearing design decisions. File references are to `src/`.

```
source text
   │  lexer.rs
   ▼
tokens (+ comments, for the formatter)
   │  parser.rs
   ▼
AST — every node carries a Span and a NodeId          (ast.rs, span.rs)
   │  check.rs  (+ types.rs, patterns.rs, builtins.rs)
   ▼
side tables keyed by NodeId: types, resolutions
   │  compiler.rs  (+ bytecode.rs)
   ▼
CompiledProgram: protos, constants, runtime type info
   │  vm.rs  (+ value.rs, natives.rs)
   ▼
execution
```

## Lexing (`lexer.rs`)

A hand-rolled scanner. The one interesting mechanism is **string
interpolation**: `"a {x} b"` lexes as `StrInterpStart("a ")`, then ordinary
expression tokens, then `StrInterpEnd(" b")`, with `StrInterpMid` between
holes. A mode stack tracks brace depth per hole, so map literals and blocks
work inside `{ … }`, and strings inside holes can themselves interpolate.
Comments are captured (not discarded) because the formatter re-emits them.

## Parsing (`parser.rs`)

Recursive descent with precedence climbing and panic-mode recovery
(synchronize at statement boundaries, guarantee token progress). Notable
disambiguations:

- `{ … }` in expression position: block or map literal? The parser
  speculatively parses `{ expr :` with full rollback (token position,
  buffered diagnostics, and NodeId counter). `{:}` is the empty map.
- `Ident { … }` is a struct literal, except in "no-struct" contexts
  (`if`/`while` conditions, `for` iterables, `match` scrutinees) where the
  brace belongs to the following block.
- `x.0.1` — the lexer produced a `Float(0.1)` token; the parser splits it
  back into two tuple indices using the token's source text.
- `|` at expression start begins a lambda; `||` there is a zero-parameter
  lambda, not the or-operator.

Every expression, pattern, statement, and type node gets a fresh `NodeId`.
The parser can start numbering at an offset (`parse_with_ids`) so REPL chunks
never collide.

## Type checking (`check.rs`)

Four passes:

1. **Predeclare types** (names + arities), then fill in field/variant types —
   so mutually recursive types resolve.
2. **Collect function signatures** — functions are hoisted; mutual recursion
   needs no forward declarations.
3. **Check top-level statements in order**, allocating global slots as `let`s
   execute. Top-level code may only reference globals declared *above* it
   (E0412), but function bodies may reference any global (reads of a global
   whose initializer hasn't run yet panic at runtime).
4. **Check function bodies**, with the fn's generic parameters held rigid.

Inference is **local unification**: `Type::Var` inference variables with a
union-find binder (`types.rs`), occurs check included. Polymorphism comes only
from explicit `[T]` parameter lists — a use of a generic function instantiates
its `Type::Param`s with fresh variables; inside its own body they are rigid
and unify only with themselves. Lambdas are checked bidirectionally: an
expected function type (from an annotation or a builtin scheme like
`List[T].map`) seeds parameter types before the body is checked, and body
constraints can also solve them (`|x| x + 1` infers `fn(Int) -> Int`).

Builtin methods live in `builtins.rs` as *type schemes* over `Param(0..)`
(receiver type arguments) and `Param(4..)` (method-own generics, e.g. the `U`
in `map`). The checker instantiates a scheme per call site and records the
resolved `Native` in the resolution table; the VM implements the same enum in
`natives.rs`. One enum, two meanings — the signature and the implementation
can't drift apart structurally.

Unsolved variables surface as targeted "cannot infer" errors at the point
that introduced them (a lambda parameter, an empty list literal, a bare
`None`) rather than at some distant use. `panic("…")` gets a polymorphic
return that defaults to `Unit` when unconstrained.

### Exhaustiveness (`patterns.rs`)

Match analysis is Maranget's usefulness algorithm over a pattern matrix.
Surface patterns lower to `DPat` (or-patterns expand to multiple rows; struct
patterns normalize to all fields in definition order). Exhaustiveness asks
"is a wildcard row useful after all unguarded arms?" — if yes, the witness it
returns is rendered into the error (`Some(false)`, `Shape.Empty`, even a
concrete uncovered integer). Reachability asks the same question per arm
against the arms before it, producing warnings. Guarded arms never count as
covering.

## Compilation (`compiler.rs`, `bytecode.rs`)

A single-pass compiler from the checked AST to a register-free stack machine.
Two mechanisms matter:

**Virtual stack depth.** The compiler simulates the stack depth at every
emitted instruction. This is what lets locals be declared *mid-expression* —
match bindings, the anonymous match scrutinee slot, and `for`-loop iterator
state are all real stack slots that coexist with expression temporaries
(`1 + match x { … }` works). Local slot = depth at declaration.

**Depth-tracked match compilation.** Each arm:

1. pre-pushes its binding slots (as `Unit`), so or-pattern alternatives and
   guards all see the same slots;
2. tests a copy of the scrutinee — navigations `Dup`/`TupleGet`/
   `GetVariantField` peel values apart, literal tests compare, bindings
   `SetLocal` into their pre-pushed slots;
3. every failing test jumps to a *failure stub* that knows exactly how many
   temporaries to pop before falling through to the next arm;
4. guards are just another failure edge (pops 0 temporaries).

Closures compile to `Closure(proto)` with upvalue descriptors resolved
lexically at compile time — capture an enclosing frame's local, or share the
enclosing closure's upvalue (transitive capture). Scope exits emit
`PopScope`/`EndBlock`, which close any upvalues pointing into the popped
range (Lua-style open→closed promotion).

The compiler is **incremental**: `ProgramBuilder` persists across REPL
chunks, appending protos and constants so indices captured by live closures
never move. Each chunk gets its own entry proto.

## Execution (`vm.rs`, `value.rs`, `natives.rs`)

`Vm::run` is a plain `match`-dispatch loop over `Op`. Values are 16-byte
tagged immediates (`Unit`/`Bool`/`Int`/`Float`/`Native`/`Obj(handle)`);
compounds live on the heap behind `u32` handles into a slot vector with a
free list. The checker guarantees operand types, so arithmetic dispatches on
runtime tags without checks beyond the guaranteed-impossible branches
(reported as "VM bug" internal errors rather than UB, should a compiler bug
ever produce them).

**GC.** Mark-and-sweep with an explicit *checkpoint* discipline:
`Heap::alloc` never collects; the VM calls `gc_checkpoint()` only at points
where every live object is rooted — before operands are popped, with natives'
arguments still on the stack, or with intermediates registered in
`temp_roots`. Roots: value stack, globals, frames' closures, open upvalues,
interned constants, cached function closures, temp roots. `FABLE_GC_STRESS=1`
turns every checkpoint into a collection, which is how the rooting discipline
is tested; `FABLE_GC_LOG=1` traces collections.

Higher-order natives (`map`, `fold`, `sort_by`, …) re-enter the interpreter
via `call_value`, which runs the dispatch loop until the frame stack returns
to its entry depth. They iterate over a temp-rooted *snapshot* of the
receiver, so callbacks that mutate the collection can't invalidate the
iteration.

**Panics** carry a message plus a stack trace assembled from the frame
stack; every instruction remembers its source span, and every proto knows its
source file, so traces have real line/column info — including through REPL
chunks.

Maps (`FMap`) are insertion-ordered vectors of `(hash, key, value)` with a
hash → indices index; structural hashing normalizes `-0.0`, is
order-insensitive for map-valued keys (matching set-like map equality), and
refuses functions. Deep equality has a nesting-depth limit so cyclic
structures (buildable via struct field mutation) error instead of hanging.

## Tooling

- **REPL** (`repl.rs`): persistent `Checker` + `ProgramBuilder` + `Vm`.
  The checker is cloned before each chunk and rolled back on error, so failed
  attempts don't pollute the session. A trailing expression is rewritten into
  a hidden global binding, then displayed with its type.
- **Formatter** (`fmt.rs`): pretty-prints the AST with literals copied
  verbatim from the source (radix and escapes survive); comments re-attach by
  original line, including same-line trailing comments; blank-line runs
  collapse to one.
- **Disassembler** (`dis.rs`): protos, constants, jump targets, and symbolic
  operands (`get_global 0 ; evens`).

## Testing strategy

- Unit tests per module (lexer shapes, parser precedence, checker
  diagnostics by error code, usefulness-algorithm cases).
- The **golden spec suite** (`tests/spec_runner.rs`): every
  `tests/spec/**/*.fable` runs in-process with captured output and asserts
  its `//? expect:` / `//? error:` / `//? panic:` directives. Tests are plain
  Fable programs — the suite doubles as a corpus of executable documentation.
- The GC stress mode exercises collector correctness on the same corpus.
