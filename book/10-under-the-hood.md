# Under the Hood

You don't need this chapter to write Fable. But if you've wondered what
happens between saving a `.fable` file and seeing output — why a captured
variable outlives its scope, or when the garbage collector actually runs —
this is the tour. Everything here was produced by running the real tools.

Fable compiles to bytecode and runs it on a stack-based virtual machine —
about 41,000 lines of dependency-free Rust in `src/`. This chapter describes
it from the outside in; pointers into the source are at the end.

## The pipeline

Every command — `run`, `check`, `dis`, `fmt`, the REPL — drives some prefix
of the same five stages:

```text
source text
    │  lexer       source → tokens (comments kept, for the formatter)
    ▼
tokens
    │  parser      recursive descent → syntax tree (AST)
    ▼
AST
    │  checker     type inference, name resolution, exhaustiveness
    ▼
typed AST
    │  compiler    one pass over the AST → bytecode
    ▼
bytecode
    │  VM          a stack machine with a mark-sweep GC
    ▼
your output
```

`fable check` stops after the checker; `fable dis` stops after the compiler
and prints what it produced; `fable fmt` only needs tokens and the AST. The
REPL keeps all five stages alive across lines, compiling each entry
incrementally into the same program.

The first two stages have debugging dumps. Given the one-line program
`let y = 3 * 4 + 1;`, `fable tokens pipe.fable` shows the lexer's view — a
flat list of tagged tokens with line:column positions:

```text
1:1	Let
1:5	Ident("y")
1:7	Eq
1:9	Int(3)
1:11	Star
1:13	Int(4)
1:15	Plus
1:17	Int(1)
1:18	Semi
2:1	Eof
```

`fable ast pipe.fable` prints the parse tree — verbose, but every node in it
carries a source span, which is how errors and stack traces later point at
exact positions.

## Reading bytecode with `fable dis`

Here is a tiny program and its complete disassembly:

```fable
let x = 3;
let y = x * 4 + 1;
println(y);
```

```text
; constants
;   [0] 3
;   [1] 4
;   [2] 1

fn <script> (proto 0, arity 0, 0 upvalues, max locals 2)
     0  const       0 ; 3
     1  set_global  0 ; x
     2  get_global  0 ; x
     3  const       1 ; 4
     4  mul
     5  const       2 ; 1
     6  add
     7  set_global  1 ; y
     8  get_global  1 ; y
     9  call_native println argc=1
    10  pop
    11  unit
    12  return
```

Things to notice:

- **Top-level code is a function too.** The script compiles to a proto
  (function prototype) named `<script>`; running a program means calling it.
- **It's a stack machine.** No registers: `const 0` pushes the constant `3`,
  `set_global 0` pops it into global slot 0 (annotated `; x` by the
  disassembler). `x * 4 + 1` becomes `get_global, const, mul, const, add` —
  operands pushed, operators pop them and push the result.
- **Builtins are direct calls.** `call_native println argc=1` needs no name
  lookup at runtime; the checker resolved it at compile time. Its `()`
  result is `pop`ped because the statement discards it.
- **Every function returns a value.** A script returns `()` — hence the
  trailing `unit; return`.

## Closures at runtime

Closures capture variables *by reference*, and captured variables outlive
their scope. The bytecode shows how. Recall the counter from chapter 3:

```fable
fn make_counter() -> fn() -> Int {
    let mut n = 0;
    || {
        n += 1;
        n
    }
}

let tick = make_counter();
println(tick());
println(tick());
println(tick());
```

It prints `1`, `2`, `3`. Here are the two interesting protos (`<script>`
omitted):

```text
fn make_counter (proto 0, arity 0, 0 upvalues, max locals 2)
     0  const       0 ; 0
     1  closure     1 ; <lambda>
     2  end_block 1
     3  return

fn <lambda> (proto 1, arity 0, 1 upvalues, max locals 2)
     0  get_upval   0
     1  const       1 ; 1
     2  add
     3  set_upval   0
     4  get_upval   0
     5  return
```

The lambda never touches a local named `n`. It reads and writes **upvalue
0** — a heap cell created when `closure 1` ran. The mechanism is borrowed
from Lua:

- When a closure captures a local, the VM allocates one *upvalue cell*
  pointing at the local's stack slot ("open"). The closure stores a handle
  to the cell, not a copy of the value.
- Two closures over the same variable get the *same cell* — that's the whole
  mechanism behind the shared `bump`/`report` counter in chapter 3. One
  variable, one cell, however many closures.
- When the scope exits, the value moves off the stack into the cell
  ("closed"). That's `end_block 1` in `make_counter`: it removes `n`'s slot
  from under the return value, closing upvalues that point into it — from
  then on the counter lives on the heap, owned by whoever holds the closure.

## How `match` compiles

There is no magic `match` instruction. A `match` compiles into a chain of
small tests with carefully bookkept jumps:

```fable
fn describe(opt: Option[Int]) -> String {
    match opt {
        Some(n) -> "got {n}",
        None -> "nothing",
    }
}

println(describe(Some(7)));
println(describe(None));
```

This prints `got 7` and `nothing`. Here is the `describe` proto (constants
and `<script>` omitted):

```text
fn describe (proto 0, arity 1, 0 upvalues, max locals 5)
     0  get_local   0
     1  unit
     2  get_local   1
     3  test_variant 0
     4  jmp_false   -> 15
     5  dup
     6  get_variant_field 0
     7  set_local   2
     8  pop
     9  const       0 ; "got "
    10  get_local   2
    11  to_string
    12  concat 2
    13  end_block 1
    14  jump        -> 28
    15  pop_n 1
    16  pop_scope 1
    17  jump        -> 18
    18  get_local   1
    19  test_variant 1
    20  jmp_false   -> 24
    21  pop
    22  const       1 ; "nothing"
    23  jump        -> 28
    24  pop_n 1
    25  jump        -> 26
    26  match_fail
    27  unit
    28  end_block 1
    29  return
```

Reading it as the compiler thinks about it:

- Instruction 0 copies the scrutinee into its own stack slot; instruction 1
  pre-pushes a `unit` placeholder slot for the binding `n`, before any
  testing happens.
- Each arm **tests a copy**: `get_local 1` pushes the scrutinee again,
  `test_variant 0` asks "is this variant 0 (`Some`)?", and `jmp_false` bails
  to the arm's **failure stub** if not. On success, `dup`/`get_variant_field`
  peel the value apart and `set_local 2` stores the payload in its slot.
- The failure stub (instructions 15–17) knows *exactly* how many leftover
  temporaries the failed tests left behind (`pop_n 1`) and discards the
  binding slots (`pop_scope 1`) before falling through to the next arm. A
  guard is one more failure edge — a stub with zero temporaries to pop.
- `match_fail` at instruction 26 aborts if execution falls off the last arm.
  The exhaustiveness checker proved it unreachable — it exists so that a
  compiler bug would fail loudly instead of corrupting the stack.

## The garbage collector

An `Int`, `Float`, or `Bool` is a 16-byte tagged value that lives directly
on the VM stack — never garbage. Everything compound (strings, lists, maps,
tuples, structs, enum values, closures, upvalue cells) is a heap object
referenced by handle.

The collector is a classic **mark-and-sweep**: starting from the roots — the
value stack, globals, call frames, open upvalue cells, constants, and a few
explicitly registered temporaries — mark everything reachable, then free the
rest. Pacing is one rule in `src/value.rs`: the first collection is offered
once the heap holds 4,096 live objects, and each collection resets the
threshold to twice the survivor count with that same floor —
`(live * 2).max(4096)`. The floor is deliberate slack: every sweep walks the
whole slot table, so a lower floor would make small working sets collect
every few hundred allocations for no real memory back, while 4,096 slots of
headroom costs a few hundred kilobytes at worst.

The design question for any GC is *when is it safe to collect?* Fable's
answer is a **checkpoint discipline**: allocating never collects. The VM
offers the collector an opportunity only at checkpoints — points where every
live object is reachable from a root (before an instruction pops its
operands, while a builtin's arguments are still on the stack). Between
checkpoints, code allocates freely; a half-built object can never be swept
out from under it.

You can watch it work. `FABLE_GC_LOG=1` logs every collection to stderr:

```fable
let mut survivors = [];
for i in 0..10000 {
    let tmp = [i, i * 2, i * 3];   // becomes garbage each iteration
    if i % 250 == 0 {
        survivors.push(tmp);       // a few lists stay reachable
    }
}
println(survivors.len());
```

```text
$ FABLE_GC_LOG=1 fable gc.fable
[gc] collected 4078 of 4096 objects (18 live, next at 4096)
[gc] collected 4062 of 4096 objects (34 live, next at 4096)
40
```

Each pass of the loop allocates one throwaway list; roughly every 4,096
iterations the heap hits its threshold and reclaims almost everything. The
live count creeps up (18, then 34) as `survivors` accumulates lists the
collector correctly refuses to touch — seventeen kept lists plus the
`survivors` list itself at the first collection, thirty-three plus one at
the second.

`FABLE_GC_STRESS=1` turns *every* checkpoint into a collection — the
harshest schedule possible. If any code path forgot to root a live object,
stress mode makes it vanish at the worst moment; that's how the rooting
discipline is tested (the whole test suite passes under it). On a tiny
program:

```fable
let words = ["once", "upon", "a", "time"];
println(words.join(" "));
```

```text
$ FABLE_GC_STRESS=1 FABLE_GC_LOG=1 fable stress.fable
[gc] collected 0 of 5 objects (5 live, next at 4096)
[gc] collected 0 of 6 objects (6 live, next at 4096)
once upon a time
```

"Collected 0" is the point: everything was rooted, so nothing was lost.

## Panics and stack traces

A panic aborts the program with exit code 70, a message, and a stack trace.
Traces are real: every compiled instruction remembers its source span, and
every proto knows which file it came from.

```fable panics
fn third(xs: List[Int]) -> Int {
    xs[2]
}

fn summarize(xs: List[Int]) -> String {
    "first={xs[0]} third={third(xs)}"
}

println(summarize([10, 20, 30]));
println(summarize([10, 20]));
```

```text
first=10 third=30
panic: list index out of bounds: index 2, length 2
  at third (trace.fable:2:5)
  at summarize (trace.fable:6:27)
  at <script> (trace.fable:10:9)
```

Note the middle frame: the panic happened inside a string interpolation, and
the trace points at the exact column of the `third(xs)` call. Builtins add
no frames of their own — a lambda passed to `map` panics like this:

```fable panics
let inverses = [4, 2, 0].map(|n| 100 / n);
println(inverses);
```

```text
panic: division by zero
  at <lambda> (divide.fable:1:38)
  at <script> (divide.fable:1:16)
```

Deep recursion panics too (`panic: stack overflow`) rather than crashing;
long traces are truncated after 64 frames. Calls in tail position reuse
their frame (see chapter 3), so only recursion with pending work — a `+`
around the recursive call, say — spends the 4,096-frame budget; give such a
function an accumulator, or reach for a loop.

## Performance

Fable is a bytecode interpreter without a JIT; it's better to know where
that ceiling is than to guess. The repo ships a micro-benchmark suite in
`bench/` — one cost centre per file, each stating exactly what it measures
in a `// Bench:` header — and `bench/run.sh` times every row against a
**release** build (a debug build is four to seven times slower and will
mislead you). One run on the author's machine — expect jitter between runs
and different absolute numbers on your hardware:

```text
$ cargo build --release
$ bench/run.sh 3
== micro (best of 3)
arith_loop               0.3953s
bench_call_return        0.2695s
bench_deque              0.2563s
bench_display            0.1656s
bench_env_maps           0.1040s
bench_join_heavy         0.1822s
bench_json               0.4837s
bench_list_churn         0.3375s
bench_lists              0.2767s
bitwise_masks            0.2479s
closure_churn            0.0732s
enum_match               0.1737s
float_loop               0.2184s
for_range                0.1682s
list_ops                 0.0888s
map_ops                  0.1131s
method_dispatch          0.0970s
string_build             0.1424s
string_interp            0.0697s
```

(The script goes on to time the heavy demo programs the same way — the
full checkers self-play game runs in about ten seconds, the Lisp
interpreter's demo session in about a second and a half.)

Some context for those numbers:

- `bench_call_return` makes 3.6 million allocation-free user-function
  calls in ~0.27 s, so a call costs well under a tenth of a microsecond —
  recursion is nothing to fear.
- `for_range` runs four million `for i in 0..N` iterations in ~0.17 s
  (~40 ns each): the range literal compiles to a single fused
  loop instruction stepping two stack slots, measurably cheaper per
  iteration than the hand-counted `while` shape `arith_loop` keeps for
  comparison. Plain `Int` and `Float` arithmetic never allocates.
- The slowest rows are honest about the costliest habits: `bench_json`
  round-trips a ~27 KB document through parse and stringify 40 times, and
  `bench_list_churn` runs 300,000 rounds of short-lived list and struct
  allocation — allocation-heavy work tops the table, arithmetic sits at
  the bottom.

For calibration rather than bragging: this is the performance class of
non-JIT scripting-language interpreters — a couple of orders of magnitude
from optimized native code, and enough to render the ray-traced scene in
`examples/raytracer.fable` in about a second.

## Where the code lives

If this chapter made you curious, the implementation is small enough to
read. Rough map, in pipeline order:

| File | What's inside |
|------|---------------|
| `src/lexer.rs` | the scanner, including the string-interpolation mode stack |
| `src/parser.rs` | recursive descent with error recovery; block-vs-map disambiguation |
| `src/check.rs` | type inference (local unification), name resolution, mutability |
| `src/patterns.rs` | exhaustiveness and reachability (Maranget's usefulness algorithm) |
| `src/compiler.rs` | AST → bytecode; the depth-tracked match compilation from this chapter |
| `src/bytecode.rs` | the `Op` enum — every instruction you saw in `fable dis`, documented |
| `src/vm.rs` | the dispatch loop, call frames, upvalues, GC checkpoints, stack traces |
| `src/value.rs` | runtime values, heap objects, and the mark-sweep collector itself |
| `src/natives.rs` | implementations of the ~250 builtin functions and methods |
| `src/dis.rs` | the disassembler (it's 124 lines — a good first file) |

`docs/ARCHITECTURE.md` covers the same ground as this chapter from a
maintainer's perspective, with the design rationale spelled out.

---

Previous: [Workers, `fft`, and the GPU](09-workers.md) ·
Next: [The Toolchain](11-toolchain.md) ·
[Back to the index](README.md)
