# 10. The Field Test

v0.5 called the language feature-complete as designed, and made one promise
about the future: anything further would be pulled in by real usage, not
pushed in by a roadmap. v0.6 is what that pull produced.

## 10.1 Ten programs walk into a language

To find out what Fable was actually like to use, ten demo programs were
written against v0.5 — each by a separate author with the same brief: build
something real, and report every place the language fights you. The results
live in `demos/`:

| Demo | What it is |
|------|------------|
| `demos/lisp` | a mini-Lisp interpreter (reader, eval, five sample programs) |
| `demos/spreadsheet` | formulas, dependency resolution, `#CYCLE!` detection |
| `demos/regex` | a backtracking regex engine built from closures |
| `demos/dungeon` | a seeded roguelike dungeon generator with BFS pathfinding |
| `demos/mdsite` | a markdown-to-HTML static site generator |
| `demos/csvql` | a query language over CSV files (`select .. where .. group by`) |
| `demos/checkers` | full English draughts with an alpha-beta engine, 106-ply self-play |
| `demos/plot` | an SVG function plotter with terminal sparklines |
| `demos/sudoku` | constraint propagation + backtracking over three classic puzzles |
| `demos/wfc` | wave-function-collapse texture generation from ASCII samples |

Every demo runs deterministically, pins its complete output with `fable test`
directives, and survives `FABLE_GC_STRESS=1`. Together they added roughly
4,500 lines of Fable — more than the interpreter's own test suite — and their
issue reports were remarkably consistent: ten authors, working blind to each
other, hit the same dozen walls. Those walls are what v0.6 removed. The full
triage — what was fixed, what was deliberately kept, and why — is in
`demos/NOTES.md`.

## 10.2 One real bug

The dungeon generator's "different seeds make different dungeons" test
failed — because it was comparing seeds 42 and 43, and `math.seed` collapsed
adjacent seeds: the old implementation set the RNG state to `seed | 1`, so
`2k` and `2k+1` produced byte-identical streams. One line of field testing
found what hundreds of interpreter tests hadn't, because the interpreter
tests all *chose* their seeds and nobody chose two in a row. v0.6 scrambles
the seed through SplitMix64:

```fable
math.seed(42);
let a = math.random();
math.seed(43);
println(a == math.random());   // false — adjacent seeds now diverge
math.seed(42);
println(a == math.random());   // true — same seed still reproduces
```

```text
false
true
```

Dice rolls also stop being an incantation: `math.rand_int(lo, hi)` is
uniform over the **inclusive** range, replacing the off-by-one-prone
`lo + (math.random() * span).to_int()` pattern every demo had hand-rolled.

## 10.3 The loops everyone wanted

Eight of ten reports asked for the same thing: `enumerate()` and `zip()`
return lists of tuples, but a `for` head only took a plain name, costing a
destructuring `let` on the first line of every loop. `for` heads now take
any irrefutable pattern:

```fable
let scores = [92, 77, 84];
for (i, s) in scores.enumerate() {
    println("player {i}: {s}");
}

let m = {"wins": 12, "losses": 3};
for (k, v) in m.entries() {
    println("{k} = {v}");
}

for _ in 0..2 {
    println("and _ discards the element");
}
```

```text
player 0: 92
player 1: 77
player 2: 84
wins = 12
losses = 3
and _ discards the element
and _ discards the element
```

`_` works as a lambda parameter for the same reason — `(0..81).map(|_| 0)`
builds a zero-filled board without inventing a name for nothing.

Match arms picked up the other ergonomic refrain: a bare `return`, `break`,
or `continue` is now a legal arm body (chapter 4 shows `return` peeling off
a failure case; `continue`/`break` arms drive loops the same way). And a
function whose body ends in `while true { ... }` with no `break` no longer
needs a dead expression after the loop — the checker knows it can only leave
via `return`:

```fable
fn first_power_over(limit: Int) -> Int {
    let mut p = 1;
    while true {
        p = p * 2;
        if p > limit { return p; }
    }
}
println(first_power_over(1000));
```

```text
1024
```

## 10.4 The string and number chores

Four demos hand-rolled a right-trim with a `slice` loop; three hand-rolled
two-decimal formatting with multiply-round-divide; two built ASCII lookup
tables to map characters to numbers. All of that is now one method deep:

```fable
println("  padded  ".trim_end() + "|");
println("  padded  ".trim_start() + "|");
println((2.0 / 3.0).to_fixed(2));
println(7.0.to_fixed(2));
println("A".code_at(0));
println(char(97));
println("abcabc".index_of_from("b", 2));
println(math.log10(1000.0));
println(math.fmod(7.5, 2.0));
```

```text
  padded|
padded  |
0.67
7.00
Some(65)
a
Some(4)
3.0
1.5
```

(`to_fixed` keeps the promised decimal places — it's for file formats and
table columns, not shortest-round-trip display — and a value that rounds to
zero prints without a stray minus sign.)

## 10.5 Sharper edges on the tools

The golden-test runner grew two precision rules after biting three demo
authors the same way: a `//?` directive only counts when it begins the
line's comment — so a comment *about* directives, or a string containing
one, no longer injects a phantom expectation — and output comparison
ignores trailing whitespace, which no editor lets you see anyway. `fable
test --help` now says so instead of silently running every test under the
current directory.

Two diagnostics got the targeted-hint treatment. The empty map:

```fable errors
let m: Map[String, Int] = {};
```

```text
error[E0301]: `{}` is an empty block, not an empty map
  note: the empty map literal is `{:}`
```

and the classic assignment-in-an-arm:

```fable errors
let mut i = 0;
match Some(3) {
    Some(v) -> i = v,
    None -> {}
}
```

```text
error[E0200]: assignment cannot be a match-arm body
  note: wrap the arm body in a block: `pattern -> { place = value; }`
```

Finally, the 4,096-frame call-depth cap — which two demos hit legitimately,
on deep spreadsheet dependency chains and long regex inputs — can be raised
per run with `FABLE_MAX_DEPTH=20000 fable run deep.fable`. The default
stays, and `try()` still catches the overflow either way.

## 10.6 What stayed out, on purpose

An honest field test also produces a list of things heard and declined —
for now, with reasons in `demos/NOTES.md`: bitwise operators (the sudoku
demo wanted bitmask candidate sets; Bool tables worked), multi-line string
literals (mdsite generated HTML line by line), a `Set` type (`Map[K, Bool]`
carried every demo that needed one), field-level visibility, and a
line-width-aware formatter. Each is real; none blocked a demo; all remain
on the same standard v0.6 was held to — they get built when usage pulls
them in, not before.

## 10.7 Where the project stands

Six releases: a language, a standard library, a toolchain, a book that
tests itself — and now a demo suite that plays checkers, solves sudoku,
renders SVG, and interprets Lisp, written in the language and pinned by the
test runner the language grew along the way. The interpreter is still
zero-dependency Rust, the spec still rules, and every claim in this chapter
ran before it was written down.

---

Previous: [The Toolchain Release](09-toolchain.md) ·
[Back to the index](README.md)
