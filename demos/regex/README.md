# regex — a backtracking regular-expression engine in Socrates

A small but complete regex engine, written to show Socrates off: enums model the
pattern AST, a recursive-descent parser turns pattern text into that AST (all
error plumbing through `Result` and `?`), and the matcher is classic
recursive backtracking in continuation-passing style — every choice point is
a closure, and the call stack doubles as the backtrack stack.

Modernized for v0.7: character classes compile to 256-bit bitmaps in a
`Bytes` buffer probed with the new bitwise operators, bounded quantifiers
`{m,n}` desugar onto the untouched matcher, and grep mode underlines *every*
match on a line through a `strings.Builder`.

## Supported syntax

| Feature | Example |
|---|---|
| literals, any-char | `cat`, `c.t` |
| quantifiers (greedy) | `x*`, `x+`, `x?` |
| bounded quantifiers (v0.7) | `x{3}`, `x{2,}`, `x{2,4}` (bounds ≤ 255) |
| alternation, grouping | `cat\|dog`, `gr(a\|e)y` |
| character classes | `[a-z0-9_]`, negated `[^0-9]` |
| anchors | `^start`, `end$` |
| shorthands | `\d \w \s` (and negated `\D \W \S`), `\.` escapes any metachar |

Matching is unanchored by default (like `grep`); use `^`/`$` to pin it down.
Strings are handled as Unicode scalars throughout, so `.` matches `é` as one
character. A `{` that does not open a well-formed bound spec is an ordinary
character: `/a{b/` matches `"a{b"`.

## Files

- `syntax.soc` — the `Ast` enum, the pattern parser (`compile`), the
  bitmap class compiler (`make_class`), the `{m,n}` desugarer (`repeat`),
  and an s-expression printer (`show`)
- `engine.soc` — the CPS backtracking matcher (`find`, `find_all`,
  `matches`)
- `main.soc` — self-checking test table, compile-error checks, bitmap
  dumps, and a tiny grep with match underlining
- `sample.txt` — a fake server log for the grep demo

## Run it

From the repository root:

```sh
# the full demo: AST dumps, 87 self-checking tests, bitmap dumps, grep examples
./target/release/socrates demos/regex/main.soc

# grep mode: filter a file (default: the bundled sample.txt) by a pattern
./target/release/socrates demos/regex/main.soc 'ERROR|WARN'
./target/release/socrates demos/regex/main.soc '\d{2,}ms$' demos/regex/sample.txt

# golden tests (the demo's whole output is pinned by //? expect: directives)
./target/release/socrates test demos/regex
```

## Sample output

```
-- how patterns compile --
  /(ab|c)*d/          =>  (seq (star (alt (seq 'a' 'b') 'c')) 'd')
  /^-?\d+(\.\d+)?$/   =>  (seq start (opt '-') (plus [0-9]) (opt (seq '.' (plus [0-9]))) end)

-- match tests --
  ok  /colou?r/            "colour"                  -> true
  ok  /gr(a|e)y/           "griy"                    -> false
  ok  /^(a|ab)*c$/         "ababc"                   -> true
  ...
summary: all 56 checks passed

-- v0.7: classes compile to 256-bit bitmaps --
  /[0-9a-f]/=>  03ff000000000000 0000007e00000000 0000000000000000 0000000000000000
  /\w/      =>  03ff000000000000 07fffffe87fffffe 0000000000000000 0000000000000000

-- v0.7: bounded quantifiers {m,n} desugar to seq/opt/star --
  /a{2,4}/            =>  (seq 'a' 'a' (opt (seq 'a' (opt 'a'))))
  ok  /^a{2,4}$/           "aaaaa"                   -> false
  ...
v0.7 additions: all 31 checks passed

grep /WARN|\d+ms/ sample.txt
   3: 2026-07-14 09:12:09 WARN  slow request GET /search 200 1450ms
                          ^^^^                               ^^^^^^
      7 of 10 lines match
```

## The v0.7 bitmap classes

The parser still produces `(lo, hi)` code-point ranges, but they are
compiled once into a `CharSet`: a 32-byte `Bytes` buffer holding one
membership bit per code point 0..255 (bit `c` lives at byte `c >> 3` under
mask `1 << (c & 7)`), plus a leftover range list for the rare class parts
above 255 (`é` fits in the bitmap; `λ` does not; `[é-λ]` straddles and is
split). The matcher's hot path is two shifts and a mask —
`bits.get(c >> 3) >> (c & 7) & 1 == 1` — however many ranges the class was
written with, and negation stays a flag consulted at match time. The demo
prints a few bitmaps as four 64-bit words, each read back out of the buffer
in one `read_u64le` (v0.8, bit 63 included) and hex-printed with `Int.to_hex`
zero-padded to 16 digits by `pad_left`.

## Bounded quantifiers

`x{m,n}` never reaches the matcher: the parser desugars it to `m` copies of
`x` followed by nested options — `x{2,4}` is `(seq x x (opt (seq x (opt
x))))`, so backtracking gives up the latest copy first — and `x{m,}` ends in
a star. Sharing one AST node between copies is safe because matching never
mutates the tree. Bounds are capped at 255 (each repeat clones a node, so an
unbounded count would be an AST bomb) and `{2,1}` is a compile error.

## Limits

Backtracking recurses once per matched character, and Socrates caps the call
stack at 4096 frames — so a repetition run of roughly 1500+ characters on one
line overflows. Grep mode wraps each line's match in `try()`, so such lines
are reported as `(skipped: stack overflow)` instead of crashing the program.
Since v0.6 you can also raise the cap for longer lines by setting the
`SOCRATES_MAX_DEPTH` environment variable (e.g. `SOCRATES_MAX_DEPTH=100000`).
Exponential patterns behave as expected for a backtracker: `(x+x+)+y` against
20 x's takes seconds; against 10 (as in the test table) it is instant.

## How the matcher works

`walk(node, chars, pos, k)` asks: *can `node` match at `pos` such that the
continuation `k` accepts the position where it ends?* Sequencing chains
continuations, alternation tries the left branch and falls back to the right
(`||` short-circuit is the backtracking), and the greedy star prefers one
more repetition before letting the rest of the pattern proceed — refusing
zero-width repetitions so patterns like `(a?)*` terminate. `find_all`
restarts the scan after each match (after the next character, for a
zero-width match), and grep's underline row is accumulated span by span in a
`strings.Builder`, whose O(1) `len()` doubles as the current column.
