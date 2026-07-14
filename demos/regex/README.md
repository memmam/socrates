# regex — a backtracking regular-expression engine in Fable

A small but complete regex engine, written to show Fable off: enums model the
pattern AST, a recursive-descent parser turns pattern text into that AST (all
error plumbing through `Result` and `?`), and the matcher is classic
recursive backtracking in continuation-passing style — every choice point is
a closure, and the call stack doubles as the backtrack stack.

## Supported syntax

| Feature | Example |
|---|---|
| literals, any-char | `cat`, `c.t` |
| quantifiers (greedy) | `x*`, `x+`, `x?` |
| alternation, grouping | `cat\|dog`, `gr(a\|e)y` |
| character classes | `[a-z0-9_]`, negated `[^0-9]` |
| anchors | `^start`, `end$` |
| shorthands | `\d \w \s` (and negated `\D \W \S`), `\.` escapes any metachar |

Matching is unanchored by default (like `grep`); use `^`/`$` to pin it down.
Strings are handled as Unicode scalars throughout, so `.` matches `é` as one
character.

## Files

- `syntax.fable` — the `Ast` enum, the pattern parser (`compile`), and an
  s-expression printer (`show`)
- `engine.fable` — the CPS backtracking matcher (`find`, `matches`)
- `main.fable` — self-checking test table, compile-error checks, and a tiny
  grep with match underlining
- `sample.txt` — a fake server log for the grep demo

## Run it

From the repository root:

```sh
# the full demo: AST dumps, 56 self-checking match tests, grep examples
./target/release/fable demos/regex/main.fable

# grep mode: filter a file (default: the bundled sample.txt) by a pattern
./target/release/fable demos/regex/main.fable 'ERROR|WARN'
./target/release/fable demos/regex/main.fable '\d\d+ms$' demos/regex/sample.txt

# golden tests (the demo's whole output is pinned by //? expect: directives)
./target/release/fable test demos/regex
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

grep /ERROR|WARN/ sample.txt
   3: 2026-07-14 09:12:09 WARN  slow request GET /search 200 1450ms
                          ^^^^
   4: 2026-07-14 09:12:11 ERROR database timeout after 3000ms
                          ^^^^^
      4 of 10 lines match
```

## Limits

Backtracking recurses once per matched character, and Fable caps the call
stack at 4096 frames — so a repetition run of roughly 1500+ characters on one
line overflows. Grep mode wraps each line's match in `try()`, so such lines
are reported as `(skipped: stack overflow)` instead of crashing the program.
Since v0.6 you can also raise the cap for longer lines by setting the
`FABLE_MAX_DEPTH` environment variable (e.g. `FABLE_MAX_DEPTH=100000`).
Exponential patterns behave as expected for a backtracker: `(x+x+)+y` against
20 x's takes seconds; against 10 (as in the test table) it is instant.

## How the matcher works

`walk(node, chars, pos, k)` asks: *can `node` match at `pos` such that the
continuation `k` accepts the position where it ends?* Sequencing chains
continuations, alternation tries the left branch and falls back to the right
(`||` short-circuit is the backtracking), and the greedy star prefers one
more repetition before letting the rest of the pattern proceed — refusing
zero-width repetitions so patterns like `(a?)*` terminate.
