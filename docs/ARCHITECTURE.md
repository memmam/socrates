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

**`gfx`** is a backend-neutral OpenGL 3.3 core-profile draw-call namespace
built on top of `window`'s per-platform `GlFns` function-pointer table
(each of `x11.rs`/`win32.rs`/`macos/gl.rs` already carries the full shader/
program/buffer/VAO/texture/uniform/draw-call table, resolved at `Window`
creation time). Two design choices make this a thin layer rather than a
new subsystem:

- **Single current context, VM-level.** `gfx.*` calls take no `Window`
  receiver — they operate against "whichever window is currently current",
  mirroring `glfwMakeContextCurrent`. `Vm` gained one field for this,
  `gfx_current_window: Option<Rc<RefCell<WindowHandle>>>`, set by the new
  `win.make_current()` method and read by every `gfx.*` native
  (`natives::gfx_window`/`gfx_window_msg`) — the same `Rc<RefCell<_>>`
  shape `Obj::Window`/`Obj::Worker` already wrap on the heap (`window_rc`/
  `worker_rc` clone it out for callers that need to release the borrow
  before blocking or recursing); `gfx_current_window` just holds one more
  clone of that `Rc`, so a window stays alive as "current" even if the
  Fable-level binding that created it goes out of scope.
- **`WindowHandle` gained one `gl_*` method per `gfx.*` member** (not one
  per raw GL entry point): `gfx.compile_program` is a single
  `WindowHandle::gl_compile_program` that internally drives
  create/compile/link/status-check/info-log/cleanup, not four separate
  wrapper calls a native would otherwise have to sequence itself. Each
  platform's `Inner` implements the same set of method names
  (`compile_program`, `bind_buffer`, `set_uniform_mat4`, ...) — mirroring
  how `poll`/`clear`/`swap_buffers` are already one name across all three
  backends — so `natives.rs` never touches a raw GL enum or FFI pointer
  type; `GfxBufferKind` (a plain two-variant enum in `window/mod.rs`) is
  the only vocabulary crossing that boundary, keeping every actual
  `GL_ARRAY_BUFFER`/`GL_ELEMENT_ARRAY_BUFFER`-style constant private to
  each platform file. Every `gl_*` method re-asserts the context is current
  before issuing GL calls (`ensure_current`), exactly like `clear`/
  `swap_buffers` already did, and no-ops (returning a zero/default) rather
  than issuing GL calls with nothing bound if that fails.

`gfx.set_uniform_mat4`'s third parameter can't be typed as `Mat4` in the
native's `NativeSig` — natives are registered before any program is parsed,
so there's no `DefId` yet for a `std` module's struct (unlike `OPTION_DEF`/
`RESULT_DEF`, which are prelude constants fixed at startup). It's typed as
a fresh scheme variable instead (the same trick `print`/`assert_eq` already
use to accept any `T`), and `natives::mat4_arg` duck-types the runtime
value's shape (a 4-field struct of 4-field-of-`Float` structs, matching
`std.glm`'s `Mat4 { c0, c1, c2, c3 }`/`Vec4 { x, y, z, w }`) — anything else
is a clear panic, not a silently wrong upload or a compile error.

`compile_program` is the one fallible member (`Result[Int, String]`);
its `Err` sizes the info-log buffer from a prior `GL_INFO_LOG_LENGTH`
query rather than guessing a fixed size, and deletes any shader/program
object already created before returning. Every other member panics
(`vm.error`, catchable via `try`) instead of threading a `Result`,
matching `window`'s own methods — two fixed messages depending on cause
(`gl` feature off at compile time, checked via `cfg!` — vs. no window
having called `make_current()` yet).

**Metal backend (macOS, additive alongside OpenGL/CGL).** CLAUDE.md carves
out a standing exception to the "newer backend supersedes the older one"
rule for Metal on macOS: it ships **additive**, never replacing
`window/macos/gl.rs`'s existing OpenGL/CGL path, unless (or until) Apple
actually drops OpenGL on macOS. A new `metal = []` feature is a sibling of
`gl`, not nested under it — the same zero-Cargo-dependency shape (raw FFI
to a system framework), independently toggleable and combinable, so
`--features gl,metal` compiles both into one binary.

**The macOS shared core (`src/objc.rs`, `src/mtl.rs`).** When the `gpu`
namespace's native Metal compute path became the *second* consumer of the
Objective-C dispatch machinery, that machinery graduated out of
`window/macos/shared.rs` into crate-level modules, per CLAUDE.md's
shared-core rule (extract at real duplication, never guess up front):
`objc.rs` holds the runtime types, `objc_msgSend`
transmute-per-shape wrappers, class/selector lookup, and the RAII
autorelease pool; `mtl.rs` holds what both Metal consumers share — the
device constructor, buffer creation, and MSL library compilation.
API-specific shapes (AppKit's window/event messages, Metal graphics'
clear-color/viewport/region calls) deliberately stay with their sole
consumers. This is the seed of the roadmap's shared graphics/compute
core: Vulkan/OpenCL/CUDA/DirectX backends grow sibling primitive modules
of the same shape as they land.

**Native Metal compute (`gpu.rs`'s `metal_native` section).** The first
native compute backend of the roadmap: `gpu.run` on Apple Silicon macOS
with `--features metal` compiles an MSL kernel (`compute_main`, buffers
0/1 — the exact WGSL contract restated in MSL) via `crate::mtl`, on a
process-lifetime device+queue pair (`OnceLock`; both objects are
documented thread-safe, so worker isolates share them soundly). The
dispatch is `(wx, wy, wz)` single-thread threadgroups, making
`thread_position_in_grid` span the same index space as WGSL's
`@workgroup_size(1)`; the output buffer is explicitly zeroed before the
dispatch (`newBufferWithLength:` guarantees nothing) so the two backends
agree on bytes the kernel never wrote; command-buffer `error` is checked
after `waitUntilCompleted`, standing in for wgpu's error scopes. Where
both `gpu` (wgpu) and `metal` are compiled in, native Metal takes
precedence — CLAUDE.md's native-backends-first rule; wgpu remains the
portable fallback until full native coverage retires it. `gpu.backend()`
(`"metal"`/`"wgpu"`/`"none"`) is the dialect escape hatch, the compute
analog of `win.backend_name()`.

Making that coexist under a single `WindowHandle` needed one structural
change: `x11::Inner`/`win32::Inner` stay plain structs, aliased directly to
`PlatformInner`, but `macos::Inner` becomes a small enum —
`Gl(gl::Inner)` / `Metal(metal::Inner)`, each variant `#[cfg]`-gated on its
own feature — because a type alias resolves to exactly one type, and only
a sum type underneath it lets one `WindowHandle` transparently hold either
a live GL-backed or Metal-backed window in the same compiled binary. Every
`Inner` method becomes a two-armed `match` forwarding to whichever variant
is live; `#[allow(clippy::large_enum_variant)]` on the enum itself
(`gl::Inner` carries the ~45-function-pointer `GlFns` table, ~456 bytes,
against `metal::Inner`'s current 0) is deliberate — boxing would add an
indirection to every hot-path `gfx.*` call for no real benefit, since at
most one `Inner` is ever live per window. `src/window/macos.rs` was split
into `macos/{mod,shared,gl,metal}.rs` to carry this: `shared.rs` holds
everything already Cocoa/AppKit-generic (`CocoaWindowState` — window
creation, the event-pump `poll()` loop, key/mouse state), which `gl.rs`'s
`Inner` now holds by composition instead of inlining.

`window.create_metal(title, w, h)` is a new **sibling** function to
`create`, not a `backend` parameter on it — Fable has neither default
parameters nor overloading, so a mandatory extra argument would break
every existing `window.create(title, w, h)` call site for no ergonomic
gain. `Window.backend_name() -> String` (`"opengl"`/`"metal"`) is the one
deliberate escape hatch this design needs: shader *source text* is
inherently backend-specific (GLSL vs. MSL), so a Fable program targeting
both backends branches on this one string and nothing else — every other
`gfx`/`Window` member keeps the exact same call shape regardless of which
backend is live. `gfx_window_msg`'s feature-off check widened from
`!cfg!(feature = "gl")` to `!cfg!(feature = "gl") && !cfg!(feature =
"metal")` — as it was, a `--features metal`-only build would wrongly
report every `gfx.*` call as "not compiled in" even with a live Metal
window current, since the check fired before ever looking at
`vm.gfx_current_window`.

`metal::Inner` holds a `CocoaWindowState` (composition, like `gl::Inner`)
plus a retained `MTLDevice`/`MTLCommandQueue`/`CAMetalLayer` and app-owned
offscreen color + depth render targets. All rendering lands in the
offscreen color texture; `swap_buffers` acquires the frame's drawable,
whole-texture-blits the target into it, presents, commits, and waits
(synchronous like a GL swap). The offscreen indirection is load-bearing,
not overhead: drawable textures are transient (no stable "back buffer"
identity across `nextDrawable` calls) and not reliably CPU-readable,
while the offscreen target uses `MTLStorageModeShared` (Apple-Silicon
uniform memory) so `read_pixels` maps it directly. One deliberate
deviation from `shared.rs`'s process-lifetime autorelease pool: the frame
path pushes/pops a pool per call, because `nextDrawable` hands back one
of a small fixed pool (~3) of drawables the layer only reclaims on actual
release — without a per-frame drain, the third `swap_buffers` would block
forever.

The `gfx.*` surface maps onto Metal as follows (all conventions
documented in SPEC § 7.4's Metal notes; `metal.rs`'s module docs carry
the full rationale): MSL sources compile via
`newLibraryWithSource:` with fixed `vertex_main`/`fragment_main` entry
points; buffers and textures live in `u32 → retained pointer` handle
tables (Metal objects are pointers, not driver-issued integers, and
handles resolve to live objects at *draw* time so `upload_buffer`'s
`glBufferData`-style same-handle-new-store semantics hold); VAOs are a
pure Rust-side shim — `set_vertex_attrib` records
`(index → size/stride/offset/buffer)` tuples exactly like GL captures
the bound array buffer into VAO state, replayed per draw as an
`MTLVertexDescriptor` (attribute `i`'s buffer bound at index `1 + i`,
index 0 reserved for the uniform struct) — and the element-array binding
is VAO state on both backends; uniforms are staged CPU-side by name and
resolved to byte offsets via pipeline reflection
(`MTLRenderPipelineReflection` → `bufferStructType` → member offsets)
at draw time, uploaded with `setVertexBytes`/`setFragmentBytes` (every
`gfx` uniform is far below Metal's 4 KiB inline limit); pipelines are
cached per `(program, vertex-layout fingerprint)` since Metal fuses what
GL binds independently; each draw is its own `loadAction=Load` render
pass into the persistent offscreen target — observably identical to GL's
free interleaving, and a candidate for the standard efficiency-pass
batching later without changing a pinned byte; `set_depth_test` swaps
between a 2-state `MTLDepthStencilState` cache (`Less`+write /
`Always`+no-write — Metal has no enable toggle); `viewport` and
`read_pixels` Y-flip internally (GL bottom-left vs. Metal top-left), and
`read_pixels` swizzles the BGRA target to RGBA and reverses rows to match
`glReadPixels`' bottom-up contract, so demos/glcube's corner-pixel pins
mean the same physical pixels on both backends.

## Testing strategy

- Unit tests per module (lexer shapes, parser precedence, checker
  diagnostics by error code, usefulness-algorithm cases).
- The **golden spec suite** (`tests/spec_runner.rs`): every
  `tests/spec/**/*.fable` runs in-process with captured output and asserts
  its `//? expect:` / `//? error:` / `//? panic:` directives. Tests are plain
  Fable programs — the suite doubles as a corpus of executable documentation.
- The GC stress mode exercises collector correctness on the same corpus.
