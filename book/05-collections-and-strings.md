# Collections and Strings

Fable ships four workhorse containers — `List[T]`, `Map[K, V]`, tuples, and
`Range` — plus a well-stocked string toolbox. This chapter tours all of
them. Every snippet is a complete program, and every output block is real
output.

## Lists

A list literal is square brackets; the element type is inferred. Lists
index from zero, know their length, and grow and shrink in place:

```fable
let primes = [2, 3, 5, 7, 11];
println(primes[0]);
println(primes.len());
primes.push(13);
primes[0] = 1;
println(primes);
println(primes.pop());
println(primes);
```

```text
2
5
[1, 3, 5, 7, 11, 13]
Some(13)
[1, 3, 5, 7, 11]
```

Two things to notice. First, `primes` is a plain `let`, yet `push` and
`primes[0] = 1` are fine — mutating a list's *contents* is not reassigning
the *binding* (the same rule as struct fields in the last chapter). Second,
`pop` returns an `Option[T]`: `Some(13)` here, `None` on an empty list.

Indexing past the end is not an `Option` — it's a panic, the runtime error
that aborts the program (exit code 70). Negative indices panic too; there
is no Python-style `xs[-1]`:

```fable
let xs = [1, 2, 3];
println(xs[3]);
```

```text
panic: list index out of bounds: index 3, length 3
  at <script> (demo.fable:2:9)
```

When an index might be out of range, ask with `get`, which returns
`Option[T]`: `xs.get(9)` is `None`, and `xs.get(9).unwrap_or(-1)` supplies
a default. `first()` and `last()` do the same for the ends. The rule of
thumb: use `[]` when an out-of-range index would be a bug you want to hear
about, and `get` when a miss is an expected case to handle.

## Lists are references

Assigning a list to a new name does not copy it — both names refer to the
same list, and mutation through one is visible through the other. When you
want an independent copy, say so with `clone`:

```fable
let xs = [1, 2, 3];
let ys = xs;          // another name for the SAME list
ys.push(4);
println(xs);

let zs = xs.clone();  // a new, independent list
zs.push(5);
println(xs);
println(zs);
```

```text
[1, 2, 3, 4]
[1, 2, 3, 4]
[1, 2, 3, 4, 5]
```

This matters most at function boundaries: a function that receives a list
receives *the* list, and any `push` it performs is visible to the caller.
`clone` is shallow — it copies the list's spine, not the elements — worth
a moment's thought for lists of structs or lists of lists.

## The transformation methods

Lists carry around thirty methods (the full inventory is in the
[spec](../docs/SPEC.md#82-listt-methods)). The daily drivers are the
transformers, which all return *new* lists. The big three — `map`,
`filter`, `fold` — chain naturally:

```fable
let scores = [72, 91, 58, 88, 96, 45];

let passing = scores.filter(|s| s >= 60);
let curved = passing.map(|s| s + 5);
let total = curved.fold(0, |acc, s| acc + s);

println(passing);
println(curved);
println(total);
```

```text
[72, 91, 88, 96]
[77, 96, 93, 101]
367
```

`fold` threads an accumulator through the list: it starts at `0`, and each
step computes the next accumulator from the current one and the element.

`sort` returns a new sorted list. It works on `Int`, `Float`, and `String`
elements and is a compile error on anything else (`error[E0322]: cannot
sort elements of type ...`); for other types, or other orders, use
`sort_by` with a comparator returning negative, zero, or positive:

```fable
let words = ["pear", "fig", "apple", "damson"];
println(words.sort());
println(words.sort_by(|a, b| b.len() - a.len()));   // longest first
println(words);                                     // untouched
```

```text
["apple", "damson", "fig", "pear"]
["damson", "apple", "pear", "fig"]
["pear", "fig", "apple", "damson"]
```

Note the quotes: strings *inside* containers display quoted, so you can
tell `["a, b"]` from `["a", "b"]`. `sort_by` is a stable merge sort:
`["fig", "kiwi", "plum", "yam"].sort_by(|a, b| a.len() - b.len())` yields
`["fig", "yam", "kiwi", "plum"]` — equal-length words keep source order.

`zip` pairs two lists element-by-element (stopping at the shorter one),
and `enumerate` pairs each element with its index. Both produce lists of
tuples:

```fable
let names = ["Gold", "Silver", "Bronze"];
let times = [9.81, 9.89, 9.94];
println(names.zip(times));
println(names.enumerate());
println(names.zip([1, 2]));
```

```text
[("Gold", 9.81), ("Silver", 9.89), ("Bronze", 9.94)]
[(0, "Gold"), (1, "Silver"), (2, "Bronze")]
[("Gold", 1), ("Silver", 2)]
```

To loop over those pairs, destructure in the body — `for pair in
names.enumerate() { let (i, name) = pair; ... }` — since `for` binds a
single name (`for (a, b) in ...` is a parse error). Finally, `flat_map`
maps each element to a *list* and splices the results into one:

```fable
let lines = ["a,b", "c", "d,e,f"];
println(lines.flat_map(|l| l.split(",")));
println([1, 2, 3].flat_map(|n| [n, n * 10]));
```

```text
["a", "b", "c", "d", "e", "f"]
[1, 10, 2, 20, 3, 30]
```

Rounding out the everyday set: `reverse`, `slice(start, end)`, and
`concat` also return new lists; `contains` and `index_of` use structural
equality (`[(1, "a")].contains((1, "a"))` is `true`); `any`, `all`, and
`find` take predicates; `each` runs a function for its side effects; and
`join` glues a `List[String]` into one string. Only strings: `[1, 2,
3].join(",")` is a compile error whose note tells you the fix — `map the
elements first: .map(|x| str(x)).join(...)`.

## Tuples

A tuple is a fixed-size group of values, possibly of different types:
`("fable", 2026, true)` has type `(String, Int, Bool)`. Access components
with `.0`, `.1`, ... or destructure the whole thing:

```fable
let entry = ("fable", 2026, true);
println(entry.0);

let (name, year, active) = entry;
println("{name} / {year} / {active}");
```

```text
fable
fable / 2026 / true
```

Unlike lists and structs, tuples are immutable *values* — `entry.0 =
"aesop"` is a compile error (`error[E0309]: tuples are immutable`), so
there's no aliasing to reason about. Tuples are the glue type: `zip`,
`enumerate`, and a map's `entries` all produce them, and they make
excellent map keys. For data with meaning beyond "these travel together,"
prefer a struct with named fields.

## Maps

A `Map[K, V]` literal looks like JSON. `m[k]` reads; `m[k] = v` inserts or
overwrites:

```fable
let ages = {"amy": 34, "ben": 27};
println(ages["amy"]);

ages["cai"] = 41;       // insert
ages["ben"] = 28;       // overwrite
println(ages);
println(ages.len());
```

```text
34
{"amy": 34, "ben": 28, "cai": 41}
3
```

One wrinkle: the empty map is spelled `{:}`, not `{}`, because `{}`
already means an empty block — `let m: Map[String, Int] = {};` is a type
error (`expected Map[String, Int], found Unit`). And since `{:}` has no
entries to infer types from, it needs an annotation or other context:

```fable
let tally = {:};
```

```text
error[E0302]: cannot infer the type of `tally`
  --> demo.fable:1:5
   |
1 | let tally = {:};
   |     ^^^^^ add a type annotation
  note: the type so far is `Map[_, _]`
```

So the idiomatic empty map is `let tally: Map[String, Int] = {:};` —
annotation on the left, colon in the braces.

Maps have reference semantics like lists (`clone` for a shallow copy), and
the same `[]`-versus-`get` split: `[]` panics on a missing key, while
`ages.get("zed")` is `None` and `ages.get("zed").unwrap_or(0)` supplies a
default:

```fable
let ages = {"amy": 34};
println(ages["zed"]);
```

```text
panic: key not found in map: zed
  at <script> (demo.fable:2:9)
```

The method forms `insert(k, v)` and `remove(k)` do the same jobs as
`m[k] = v` and deletion but return the previous value as an `Option[V]` —
handy when you need to know whether the key was already there.

### Insertion order

Iteration order is *insertion order*, deterministically — not the
arbitrary scramble of many hash-map implementations. Overwriting a key
keeps its position; removing and re-inserting moves it to the end:

```fable
let m: Map[String, Int] = {:};
m["zebra"] = 1;
m["aardvark"] = 2;
m["mole"] = 3;
m.remove("aardvark");
m["aardvark"] = 4;      // re-inserted: goes to the end
println(m.keys());
println(m.entries());
```

```text
["zebra", "mole", "aardvark"]
[("zebra", 1), ("mole", 3), ("aardvark", 4)]
```

`keys()`, `values()`, and `entries()` return lists — which is also how you
loop over a map, since `for` iterates lists, ranges, and strings: `for
entry in ages.entries() { let (name, age) = entry; ... }`.

### Structural keys

Keys are compared and hashed *structurally*, so they don't have to be
strings — any value works, including tuples. A `Map[(Int, Int), V]` is a
sparse 2-D grid with no encoding tricks:

```fable
let board: Map[(Int, Int), String] = {:};
board[(0, 0)] = "rook";
board[(4, 7)] = "queen";

println(board[(0, 0)]);
println(board.get((3, 3)));
println(board);
```

```text
rook
None
{(0, 0): "rook", (4, 7): "queen"}
```

The one exclusion: values containing functions can't be keys (there is no
sensible equality for closures). The compiler rejects it when the key type
is written out, and the runtime panics if one sneaks in through a generic.

## Ranges

You met `a..b` (half-open) and `a..=b` (inclusive) as `for`-loop fodder in
chapter 2, but ranges are ordinary values of type `Range` — bind them,
pass them, call methods on them. `map`, `filter`, and `fold` work directly
on a range and produce lists:

```fable
let r = 1..=5;          // ranges are ordinary values
println(r.to_list());
println(r.contains(5));
println((1..5).contains(5));
println((1..=10).filter(|n| n % 3 == 0));
println((1..=4).map(|n| n * n));
println((1..=5).rev());
```

```text
[1, 2, 3, 4, 5]
true
false
[3, 6, 9]
[1, 4, 9, 16]
[5, 4, 3, 2, 1]
```

Endpoints are always `Int`. `rev()` returns a reversed *list*, so counting
down is `for i in (1..=5).rev()`, and `(0..n).map(...)` is the cheap way
to conjure an indexed list — see the sieve in
[`examples/algorithms.fable`](../examples/algorithms.fable).

## Strings

Strings are immutable UTF-8, counted in Unicode scalars — "characters" for
everyday purposes — not bytes. `len` is the character count, `byte_len`
the storage size, and `chars` explodes a string into a list of
one-character strings (there is no separate character type):

```fable
let word = "héllo";
println(word.len());
println(word.byte_len());
println(word.chars());
```

```text
5
6
["h", "é", "l", "l", "o"]
```

Because character counts and byte counts disagree, `s[0]` would be
ambiguous bait, so it's simply not allowed — the compile error
(`error[E0313]: strings cannot be indexed with []`) points you at
`.chars()`, `.char_at(i)`, or `.slice(a, b)` instead.

The everyday toolbox — trimming, case, searching, replacing, padding — in
one pass:

```fable
let s = "  The Tortoise and the Hare  ";
println(s.trim());
println(s.trim().to_upper());
println(s.contains("Tortoise"));
println(s.replace("Hare", "Snail").trim());
println("ab".repeat(3));
println("7".pad_left(3, "0"));
```

```text
The Tortoise and the Hare
THE TORTOISE AND THE HARE
true
The Tortoise and the Snail
ababab
007
```

`starts_with`, `ends_with`, and `pad_right` round out the set. One honest
limitation: `to_upper` and `to_lower` map ASCII letters only —
`"étude".to_upper()` is `"éTUDE"`, the `é` passing through unchanged. Full
Unicode case mapping is out of scope for v0.1.

### Splitting

`split` has the edge cases you'd hope for, plus one convention to
memorize: adjacent (or leading/trailing) separators produce *empty
strings*, not nothing. That's what lets `split` round-trip with `join`,
and it matches Rust:

```fable
println("a,b,c".split(","));
println("a,,c".split(","));       // adjacent separators keep the empty field
println(",a,".split(","));        // ...as do leading/trailing ones
println("abc".split(""));         // empty separator splits into chars
println("no-commas".split(","));
```

```text
["a", "b", "c"]
["a", "", "c"]
["", "a", ""]
["a", "b", "c"]
["no-commas"]
```

Parsing sloppy input and want the empty fields gone? That's a one-liner:
`s.split(",").filter(|f| !f.is_empty())`.

### Slicing and searching

`slice(start, end)` takes character indices, half-open like ranges, and
*clamps* out-of-range ends instead of panicking. `char_at` and `index_of`
return `Option`s, and `index_of` reports a character index, consistent
with everything else:

```fable
let s = "collections";
println(s.slice(0, 7));
println(s.slice(7, 999));        // out-of-range ends are clamped
println(s.char_at(0));
println(s.char_at(99));
println(s.index_of("lect"));
```

```text
collect
ions
Some("c")
None
Some(3)
```

### Parsing numbers

`parse_int` and `parse_float` return `Option`s rather than panicking on
bad input. `parse_int` is strict — an optional sign, decimal digits,
nothing else — while `parse_float` accepts the usual notations including
exponents:

```fable
println("42".parse_int());
println(" 42 ".parse_int());     // no surrounding whitespace allowed
println("0x2A".parse_int());     // decimal only
println("3.5".parse_int());
println("3.5".parse_float());
println("1e-3".parse_float());
```

```text
Some(42)
None
None
None
Some(3.5)
Some(0.001)
```

For messy input, `.trim().parse_int()` handles the whitespace case, and
`unwrap_or` supplies the default.

## Building strings: collect, then join

Strings are immutable, so `a + b` allocates a fresh string. Grow a string
with `+=` in a loop and every pass re-copies everything built so far —
quadratic work. The idiom for building a big string is to push the pieces
onto a `List[String]` and `join` once at the end. The difference is not
academic — here are both strategies building the same 80,000-piece
string, timed with the builtin `clock()`:

```fable
let n = 80000;

let t0 = clock();
let mut slow = "";
for i in 0..n {
    slow += "{i},";
}
let concat_ms = ((clock() - t0) * 1000.0).round();

let t1 = clock();
let parts: List[String] = [];
for i in 0..n {
    parts.push("{i},");
}
let fast = parts.join("");
let join_ms = ((clock() - t1) * 1000.0).round();

assert_eq(slow, fast);
println("concat: {concat_ms}ms");
println("join:   {join_ms}ms");
```

```text
concat: 1163.0ms
join:   101.0ms
```

Your numbers will differ, but the shape won't: doubling `n` roughly
doubles the `join` time and roughly *quadruples* the `concat` time. For a
handful of pieces, `+` and interpolation are fine; reach for the list when
building strings in a loop. (`join` puts the separator only *between*
elements: `["solo"].join(" and ")` is `"solo"`, and joining an empty list
is `""`.)

## Putting it together

Word frequency is the classic collections kata, and it uses one idiom from
each section — `split` to tokenize, `get(...).unwrap_or(0)` to count into
a map, `entries` to get the map back out, a stable `sort_by` to rank, and
`slice` to take the top three:

```fable
let text = "the quick brown fox jumps over the lazy dog";

let counts: Map[String, Int] = {:};
for word in text.split(" ") {
    counts[word] = counts.get(word).unwrap_or(0) + 1;
}

let ranked = counts.entries().sort_by(|a, b| b.1 - a.1);
for entry in ranked.slice(0, 3) {
    println("{entry.1}x {entry.0}");
}
```

```text
2x the
1x quick
1x brown
```

Because map iteration is insertion order and `sort_by` is stable, this
prints the same thing every run — ties rank in the order the words first
appeared.

## Where we are

Lists and maps are mutable reference types — `clone` when you want a copy,
`get` when a miss is expected, `[]` when it would be a bug. Tuples are
immutable glue, ranges are values, and strings count characters, not
bytes, and are built efficiently with collect-then-join. Between the
containers and the `Option`-returning methods everywhere, most day-to-day
Fable is a transformation pipeline ending in a pattern match. Next: under
the hood, at the bytecode and garbage collector that make it all go.
