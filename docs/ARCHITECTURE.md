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

## v0.2 additions

**impl blocks.** A method is an ordinary checker function whose first
parameter is the receiver: the parser synthesizes `self`'s `TypeExpr`
(`TypeName[G, ...]`) so methods run through the same signature-collection,
body-checking, and proto-compilation paths as free functions, registered
under a mangled name (`Point.len`) and indexed by `(DefId, method name)`.
Method-call dispatch consults that map after resolving the receiver type,
ahead of the builtin `Recv` tables; calls lower to `CallFn` with the
receiver as argument 0. No VM changes at all.

**`?` operator.** `ExprKind::Try` compiles to
`TestVariant(0); JumpIfFalse fail; GetVariantField(0); Jump end; fail: Return`
— `Some`/`Ok` are variant 0 of their prelude enums and the failure value on
the stack *is* the propagated return value, so the fail path is a bare
`Return`. The checker unifies the enclosing return type with
`Option[fresh]`/`Result[fresh, E]` per the operand.

**Tail calls.** The compiler threads tail position from `return` operands,
function/lambda body tails, and `if`/`match`/block result positions, then
rewrites a just-emitted `Call`/`CallFn` into `TailCall`/`TailCallFn`
in place — the tail variants have the same operand shapes and stack effects,
so no jump offsets or depth bookkeeping move. In the VM, `reuse_frame`
closes the departing frame's upvalues, slides the callee slot and arguments
down over it, and resets the frame in place; a native value in tail
position is called and its result returned directly.

**Modules.** `modules.rs` DFS-loads imports (file-relative paths,
canonical-path dedup for diamonds, cycle detection), parsing each file with
offset NodeIds — the same trick the REPL uses — so all modules share one
checker. Each module checks under a name-mangling prefix: its top-level
names register as `"key.name"` in the existing fn/global/def tables, and
every unqualified lookup qualifies with the current module's prefix (the
prelude `Option`/`Result` stay visible everywhere). Qualified references
resolve through the module's alias map; module function calls get their own
`Res::ModuleFn` so the compiler knows not to push a receiver. Compilation
reuses `ProgramBuilder::compile_chunk` per module (again the REPL path),
and the VM runs each module's script proto in dependency order over shared
globals — `run_entry_at` is the only VM addition.

## v0.3 additions

**Visibility.** `pub` flags travel on FnInfo/GlobalInfo/TypeDef (methods are
FnInfos, so per-method visibility came free). Enforcement lives exactly at
the foreign-naming choke points added for modules: the import-qualified
lookup paths and cross-module method dispatch (a stored name's module is its
prefix before the last dot). Structural use of foreign values — field reads,
type-directed patterns — is deliberately ungated.

**Operator methods.** `check_binary` intercepts before unifying operand
types: a `Named` left operand with the operator's well-known method (`add`,
`sub`, `mul`, `div`, `rem`; `neg` for unary minus) checks like a one-argument
method call and records `Res::Fn` on the operator node; the compiler emits a
plain `CallFn`. No new ops, no VM changes. Compound assignment and `==` are
deliberately excluded.

**Module search path.** The loader takes an ordered dir list (file-relative
first, then `FABLE_PATH` entries); canonical-path dedup already made
same-file-through-different-bases safe.

**fs/os.** The `math` namespace machinery generalized to a
`namespace_member(ns, member)` table; the new natives follow the existing
rooting discipline (`alloc_rooted_list`/`temp_roots`) and return
`Result[_, String]` so failures compose with `?`. The VM carries
`script_args` for `os.args()`.

## v0.4 additions

**fable test** is the spec harness moved into the library (`testing.rs`);
the CLI command and the Rust test suite call the same functions.

**The std library** is Fable source embedded with `include_str!`
(`stdlib.rs`, `std/*.fable`). The loader intercepts the reserved `std.`
prefix before any filesystem resolution, keys the modules by pseudo-paths
in the same dedup/cycle maps, and forbids std modules from importing
anything non-std. From the checker's perspective a std module is just a
module.

**The language server** (`lsp.rs` + `jsonlite.rs`) reuses the whole
pipeline: the loader gained an overlay parameter so the unsaved buffer
shadows the on-disk file, and analysis is simply load + check per module,
retaining the checker for hover (`types` by NodeId; smallest-span node
lookup is a ~100-line AST walk) and definition (the `res` table; a stored
name's prefix locates the defining module's file). Positions are converted
properly between byte offsets and LSP's UTF-16 line/character pairs.

**try(f)** snapshots frame/stack/root depths, delegates to `call_value`,
and truncates back on error — the same unwinding discipline `run_entry`
already used for the REPL.

**GC pacing**: the next-collection threshold's floor rose from 256 to
4,096 live objects. Small working sets used to collect every ~250
allocations, and each sweep walks the entire slot table; the closure-churn
benchmark ran 1,604 collections (161ms), now 98 (38ms), with allocation
also cheapened by an early-out in `close_upvalues` and a clone-free
`Op::Closure` path.

## v0.5 additions

**REPL imports**: a `ModuleSession` persists the loader's dedup and key
state across chunks; each chunk's imports load against the working
directory / `FABLE_PATH` / `std.`, the new units check and run before the
chunk, and both the checker and the session roll back together on failure.

**Completion** reads the receiver chain from the *current* buffer text and
resolves it against the last analysis whose load succeeded — so it works
while the buffer doesn't parse. The builtin method registry became one
const table serving lookup and enumeration.

**The executable book** (tests/book_snippets.rs): fence tags classify the
deliberate-failure demos (`errors`/`panics`), support-file blocks are
written into a per-chapter directory for real multi-file imports, and
directive-bearing blocks run under `fable test` semantics.

## v0.6 additions

The field-test release: ten demo programs (`demos/`) were written against
v0.5 and their issue reports triaged into fixes (`demos/NOTES.md` is the
ledger). The mechanically interesting parts:

**`for` patterns**: `StmtKind::For` now carries a `Pattern` instead of an
`Ident`. The checker reuses the `let` machinery verbatim
(`check_pattern` → `assert_irrefutable` → `materialize_binds`, inside the
loop's scope, so bindings are always locals). The compiler binds the
element ForNext pushes via `bind_loop_pattern`: the single-name fast path
declares the slot in place; destructuring keeps the element as an
anonymous local and extracts each binding by navigation path, exactly like
`let` (`collect_bind_paths`). Everything lives in the body scope, so the
per-iteration unwind and `break`/`continue` depth logic are unchanged.

**Divergence**: `check_block` already typed a trailing
`return`/`break`/`continue` as a fresh defaulting variable; a trailing
`while true { .. }` now joins it when `block_contains_break` says the loop
cannot fall through. The break-scan deliberately **over-approximates**
(it descends into nested loops and lambdas where a `break` couldn't target
this loop) because a false "contains break" merely reverts to the old
typing, while a false "no break" would be unsound. `os.exit` switched its
scheme to `panic`-style (`ret = Param(0)`), which is the whole change.

**Match-arm statement sugar**: the parser desugars `-> return x` /
`-> break` / `-> continue` into a one-statement block body, so the
checker's divergence rule, the compiler, and the formatter all see a shape
they already handle (fmt canonicalizes the sugar to the block form).

**RNG**: `math.seed` collapsed adjacent seeds — state was `seed | 1`, so
2k and 2k+1 were identical streams (found by the dungeon demo's
different-seeds test). Seeds now pass through SplitMix64; `rand_int` uses
the widening-multiply reduction over a raw `u64` draw.

**Directive scanner**: `//?` only counts when it begins the line's comment,
with just enough string-awareness to skip `//` inside quotes; golden
comparison ignores trailing whitespace on both sides. Three demo authors
were bitten by prose *about* directives becoming directives.

New builtins (`trim_start`/`trim_end`/`code_at`/`index_of_from`,
`char`, `to_fixed`, `math.rand_int`/`log10`/`fmod`) follow the existing
pattern: one enum variant, one `sig()` scheme, one `METHOD_TABLE` row (the
LSP completes them for free), one `natives.rs` arm. `FABLE_MAX_DEPTH` is
read once at VM construction into a `max_frames` field.

## v0.7 additions

**Bytes and bitwise.** `Obj::Bytes(Vec<u8>)` is a new GC leaf (no traced
children) with a `Type::Bytes` primitive; its methods (checked accessors,
little- and big-endian pushers/readers, bulk appends, UTF-8 bridging) follow
the one-variant/one-`sig()`/one-`METHOD_TABLE`-row/one-`natives.rs`-arm
pattern, and `fs.read_bytes`/`write_bytes` move it to disk. Bitwise
operators (`& | ^ << >>`) are Int-only ops in the compiler/VM with Rust's
relative precedence; the Int intrinsics (`count_ones`/`ushr`/`rotate_*`/
`to_hex`, plus Bytes readers) are ordinary natives, and the demos' formerly
hand-rolled versions are now one-line wrappers over them.

**fft.rs** implements the `fft` builtin namespace — an iterative radix-2
Cooley–Tukey transform for power-of-two lengths and Bluestein's chirp-z for
everything else, over split-complex `List[Float]` pairs, following numpy's
conventions. It is pure Rust with a naive-DFT oracle in its unit tests and a
CI cross-check against numpy at 1e-9; the natives marshal the lists in and
out with the standard rooted-allocation helpers.

**worker.rs** implements worker isolates: `worker.spawn` compiles a file into
a brand-new `Vm` (its own heap, globals, and GC) on its own OS thread, joined
to the parent only by `String` channels (`std::sync::mpsc`). Nothing GC'd is
shared, so no locking is needed; the parent's handle is a non-traced
`Obj::Worker` holding the channel ends and the join handle. `spawn`
handshakes on a compile result so errors surface synchronously, and a
worker's panic is caught at its thread boundary and returned as an `Err` from
`join`. Worker `println` output is routed through a shared sink so the
golden-test harness can capture it.

**bundle.rs** packs a program into a self-contained binary for `fable build`.
It serializes every file under the program's directory into a
dependency-free little-endian archive and `staple`s it onto interpreter bytes
as `payload ‖ u64(len) ‖ MAGIC`; the launcher entry in `main.rs`
(`run_bundle`) calls `read_self`, which seeks to the executable's last 16
bytes and, only if the magic matches, reads back the payload — so an ordinary
`fable` pays one 16-byte read and nothing else. A present bundle is extracted
into a per-process scratch directory that becomes the working directory, then
the normal file-path runner takes over: because files are packed under the
path *as given* to `build`, imports, `fs.*`, and `worker.spawn` all resolve
against the unpacked tree with no special-casing anywhere in the loader or the
VM. Extraction refuses absolute or `..` paths. Because stapling is pure byte
concatenation and target-independent, `--launcher` lets one host assemble
binaries for every cross-compiled target (the release "demo zoo"). macOS is
the exception to the append: a Mach-O with data past `__LINKEDIT` fails code
signing (and Apple Silicon won't run unsigned), so there the payload is linked
in as a `__DATA,__fablezoo` section (`fable build --payload-only` emits the
raw archive; the release links it with `ld -sectcreate`). `read_self` handles
both — tail magic first, then a portable Mach-O parse for the section, then a
backward scan tolerating a trailing code signature.

**The efficiency pass** rewrote the interpreter's hot paths against a
benchmark harness (`bench/`, results in `bench/RESULTS.md`): dispatch-loop
state hoisted into `run()` locals, write-in-place stack traffic,
allocation-free `for` over Int ranges, an allocation-free GC mark phase,
FMap single-entry index buckets without SipHash, borrow-based string/list
natives, and `strings.Builder` re-backed by a `Bytes` buffer. Every change
kept observable output byte-identical and was gated on interleaved A/B
measurement.

**gpu.rs** is the home of the `gpu` builtin namespace's implementation and
the project's only dependency boundary: wgpu (+ pollster) sit behind the
`gpu` cargo feature, so the default build stays zero-dependency (CI asserts
`cargo tree` is one line). The module is always compiled — only its
internals are `#[cfg]`-gated — so `builtins.rs`/`natives.rs` register the
natives unconditionally and feature-off builds degrade gracefully
(`available()` false, `run()` an `Err`) instead of failing to resolve
`gpu.*`. `run` is synchronous from the VM's point of view (pollster blocks
on wgpu's futures), copies its input out of the heap before any device work
and allocates the result after all of it, so the GC checkpoint discipline is
untouched. wgpu reports validation failures (shader compile errors included)
asynchronously; `run` brackets all device work in error scopes and converts
anything captured into the `Err` message. `run`'s I/O rides on the v0.7
`Bytes` heap object (`Obj::Bytes`, a GC leaf) introduced with the binary-I/O
work — the gpu natives reuse its existing helpers in `natives.rs`.

## v0.8 additions

**`if let` / `while let`** are parser-only sugar, desugared fully to
existing AST at parse time — no new bytecode ops, and no new cases in
`check.rs` or `compiler.rs`. `if let PAT = E { T } else { F }` builds an
ordinary two-arm `ExprKind::Match` (`PAT -> T`, a synthetic `_ -> F`, or `_
-> Unit` with no `else`); `while let PAT = E { B }` builds `StmtKind::While
{ cond: true, body: [PAT -> B, _ -> break] }` — textually the exact
hand-rolled idiom STYLE.md already documented, so no new compiler logic is
needed: it's an ordinary `Match` (generic arm-body compilation) nested in an
ordinary `While` (generic loop compilation), and `break` inside a match arm
already compiles correctly because match arms are compiled as regular
expressions/blocks, unrelated to which construct encloses them. A new
`ExprKind::Match` field, `sugar: MatchSugar` (`None`/`IfLet`/`WhileLet`), is
purely for two consumers: `fmt.rs` prints the sugar back instead of the
desugared `match`/`while true`, and `check.rs`'s deferred reachability pass
(`analyze_matches`) skips the "unreachable match arm" warning on a sugar
match's synthetic fallback arm — an irrefutable user pattern makes that arm
genuinely unreachable, but the user never wrote it, so warning on it would
point at invisible compiler-generated code. One correctness subtlety: the
desugared `while let`'s `Match` statement must be the loop body block's
*tail* expression (`StmtKind::Expr { tail: true }`), not a bare `tail:
false` statement — otherwise `check.rs`'s existing `expect_unit_body` call
on the `While`'s body block never sees the match's type, silently bypassing
the same "loop body must not produce a value" check (E0306) that a
hand-written `while`/`for` already gets.

**Bitwise compound assignment** (`&= |= ^= <<= >>=`) reuses the existing
`StmtKind::Assign { target, op: Option<BinOp>, value }` machinery wholesale
— `lexer.rs` grows five tokens (disambiguated the same way `&&`/`||`
already are: `&` then peek `&` then peek `=`, etc.), `parser.rs` maps them
to `BinOp::BitAnd/BitOr/BitXor/Shl/Shr` in the same match arm that handles
`+=`/`-=`/etc., and `compiler.rs`'s `bin_op()` already had cases for those
`BinOp` variants (from the v0.7 binary bitwise operators), so codegen needed
no changes at all. The one real change is in `check.rs`'s
`check_arith_operand`: the bitwise variants previously fell into its
permissive `_ => true` catch-all (meaning a compound bitwise assign on a
non-`Int` would have silently type-checked), so they're now routed to the
same `Int`-only rule the plain bitwise binary operators already enforce.

**Hex/binary literal bit patterns.** `lexer.rs`'s `radix_number` parsed via
`i64::from_str_radix`, rejecting any pattern needing bit 63 (`>=
2^63`) even though `to_hex()`/the bitwise operators already treat `Int` as
a raw 64-bit two's-complement value. Switched to `u64::from_str_radix` then
`as i64` (a bit-pattern reinterpret, not a range-checked parse) — decimal
literals are untouched, still `i64`-range-checked. `String.parse_hex()` is
the same reinterpret in the other direction (`u64::from_str_radix` on the
string minus an optional `0x`/`0X` prefix, then `as i64`), a new native
alongside `parse_int`/`parse_float`.

**Bytes 64-bit accessors and wrapping arithmetic** are ordinary natives
following the established one-variant/one-`sig()`/one-`METHOD_TABLE`-row
pattern. `push_u64le`/`be` need no range check (unlike the 16/32-bit
pushers) since `Int` already occupies exactly 64 bits — every value is
representable; the reads (`read_u64le`/`be`) reinterpret the 8 bytes as
`i64` directly, so unlike the 32-bit reads they can come back negative.
`wrapping_add`/`sub`/`mul` are direct `i64::wrapping_*` calls — deliberately
64-bit only (Rust's own naming), since a 32-bit wrap is one mask away
(`a.wrapping_mul(b) & 0xFFFFFFFF`) rather than a second intrinsic.
`fft.magnitude` and `Range.any`/`all` (the latter short-circuiting, mirroring
`ListAny`/`ListAll`) and `worker.try_recv` (a `TryRecvError`-to-nested-`Option`
translation on the same `mpsc::Receiver` both `WorkerHandle::recv` and
`WorkerCtx::rx` already wrap) are the same pattern again.

**`std.lazy`, `std.json` construction, `Builder`/`lists` ergonomics** are
pure Fable — no Rust changes beyond `stdlib.rs`'s embedded-module table
gaining `std.lazy`. `Lazy[T]` holds `value: Option[T]` and `thunk: fn() ->
T`; calling a function stored in a field requires binding it to a local
first (`let f = self.thunk; f()`), since `self.thunk()` parses as a method
call attempt and fails to resolve — the same pattern `std.iter`'s
`Iter[T].next` already uses. `std.json`'s new constructors are named
`jstr`, not `str`, because a module-local `pub fn str` would shadow the
prelude's `str()` for every unqualified call inside that same file (it did,
breaking `num_str`'s internal `str(f.to_int())` call, until renamed).

**`fable test --bless`** (`testing.rs`): `check_one` is now a thin wrapper
over `check_or_bless(path, bless: bool)`. On a stdout-only mismatch, when
`bless` is set and the actual/expected line counts already agree, it
re-scans the file with the same `directive_start` line scanner
`parse_directives` uses, replaces the *n*-th `//? expect:` line's payload
with the *n*-th actual output line in file order, and writes the file back
— a line-count mismatch (a print statement added or removed) returns `Ok`
untouched, leaving the original mismatch as a normal failure. `error:`/
`panic:` directives are never rewritten.

## Testing strategy

- Unit tests per module (lexer shapes, parser precedence, checker
  diagnostics by error code, usefulness-algorithm cases).
- The **golden spec suite** (`tests/spec_runner.rs`): every
  `tests/spec/**/*.fable` runs in-process with captured output and asserts
  its `//? expect:` / `//? error:` / `//? panic:` directives. Tests are plain
  Fable programs — the suite doubles as a corpus of executable documentation.
- The GC stress mode exercises collector correctness on the same corpus.
