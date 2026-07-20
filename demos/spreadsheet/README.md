# spreadsheet — a formula engine in Socrates

A small spreadsheet: cells hold numbers, words, or formulas; formulas are
parsed with a Pratt parser, evaluated with memoization and cycle detection,
and the result is rendered as an aligned ASCII grid.

Formulas start with `=` and support:

- numbers (`=1.5`), cell references (`=B2`), rectangular ranges (`B2:D5`)
- `+ - * /`, parentheses, unary minus, with the usual precedence
- `sum`, `avg`, `min`, `max`, `count` over any mix of ranges and scalars
  (aggregates skip words and blank cells, like a real spreadsheet)

Aggregates follow the identity rule: `sum` and `count` of an empty range
are `0`, while `avg`/`min`/`max` need at least one number and turn an
empty range into `#VALUE!` (std.lists' `min_float`/`max_float` return
`Option`, and `None` maps straight onto the error). Any error cell in any
argument wins over everything else — first one in argument order, row-major
within a range. `aggregates.soc` + `stats.sheet` pin all of this.

Bad cells become error *values* instead of crashing the sheet: `#CYCLE!`
(reference cycles, found by the busy-set walk in `sheet.soc`), `#DIV/0!`,
`#VALUE!` (words in arithmetic), `#NAME?` (unknown function), `#REF!`
(address off the grid), and `#PARSE!` (formula didn't parse). Errors
propagate through every formula that reads them.

## Run it

From the repository root:

```sh
./target/release/socrates demos/spreadsheet/main.soc                # both demo sheets
./target/release/socrates demos/spreadsheet/main.soc my.sheet       # your own file
./target/release/socrates demos/spreadsheet/aggregates.soc          # aggregate edge cases
./target/release/socrates test demos/spreadsheet                      # golden tests
```

Sheet files are CSV-ish: one line per row, commas between columns, `#`
comment lines, and commas inside parentheses belong to the formula
(`=sum(1,2,3)` is one cell).

## Files

| File               | What it is                                              |
|--------------------|---------------------------------------------------------|
| `formula.soc`    | lexer + Pratt parser producing the `Expr` tree          |
| `sheet.soc`      | grid model, loader, evaluator (cycle detection), renderer |
| `main.soc`       | CLI glue, plus the golden `//? expect:` output          |
| `checks.soc`     | unit-style golden tests against the public API          |
| `aggregates.soc` | min/max/avg on empty, words-only, and error-poisoned ranges |
| `budget.sheet`     | the happy-path demo sheet                               |
| `cycles.sheet`     | the unhappy-path demo sheet                             |
| `stats.sheet`      | the aggregate torture sheet (`aggregates.soc` renders it) |

The engine leans on the v0.7 collections layer: the cycle-detection busy
set is a `std.set` (one `insert()` both marks the cell and detects the
cycle — `false` means "already being evaluated", i.e. `#CYCLE!`), the
aggregates are `std.lists` (`sum_float`, `min_float`, `max_float`), and
the grid and formula report accumulate in a `strings.Builder` instead of
a list-of-lines join.

## Sample output

```
=== demos/spreadsheet/budget.sheet
   |   A    |  B  |     C     |   D
---+--------+-----+-----------+------
 1 | Item   | Qty | Price     | Total
 2 | Apples |   4 |       1.5 |     6
 3 | Bread  |   2 |      3.25 |   6.5
 4 | Cheese |   1 |       7.9 |   7.9
 5 | Milk   |   3 |      0.95 |  2.85
 6 |        |     | Subtotal  | 23.25
 7 |        |     | Tax (8%)  |  1.86
 8 |        |     | Total     | 25.11
 ...

=== demos/spreadsheet/cycles.sheet
   |         A          |    B
---+--------------------+--------
 1 | case               | result
 2 | three-cell cycle a | #CYCLE!
 3 | three-cell cycle b | #CYCLE!
 4 | three-cell cycle c | #CYCLE!
 5 | reads the cycle    | #CYCLE!
 6 | self reference     | #CYCLE!
 7 | divide by zero     | #DIV/0!
 ...
```
