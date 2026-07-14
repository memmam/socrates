# csvql — a mini query engine over CSV, in Fable

csvql loads a CSV file into a typed in-memory table (each cell becomes a
number or text), parses a small SQL-ish query language, executes it, and
prints the result as an aligned ASCII table. The bundled data set is
`cities.csv`: 27 world cities with metro population (millions) and
elevation (metres).

The query language:

```
select <cols>                          -- columns, '*', count, sum/avg/min/max <col>
  [where <col> <op> <value> [and ...]] -- op: == != < <= > >= contains
  [group by <col>]
  [order by <key> [asc|desc]]          -- key may be an aggregate: 'order by avg pop'
  [limit <n>]
```

Values in `where` are numbers, bare words, or `'single quoted text'` (for
values with spaces: `where country == 'United States'`). Operators need
surrounding spaces. Without `group by`, aggregates fold the whole result
into one row; with it, `order by` names an *output* column.

## Run it

From the repo root:

```
./target/release/fable demos/csvql/main.fable            # 8 showcase queries
./target/release/fable demos/csvql/main.fable \
    "select city, pop where continent == Asia order by pop desc limit 3"
./target/release/fable test demos/csvql                  # golden tests
```

Every argument is run as its own query against `cities.csv`.

## Sample output

```
csvql> select continent, count, avg pop group by continent order by avg pop desc
continent     | count | avg pop
--------------+-------+--------
Asia          |     7 |   25.47
South America |     4 |   15.08
Africa        |     4 |   14.78
North America |     5 |   13.78
Europe        |     6 |    8.03
Oceania       |     1 |     5.1
(6 rows)

csvql> select city where fame > 9000
error: unknown column 'fame' (columns: city, country, continent, pop, elev)
```

Bad queries never crash: parsing and execution both return
`Result[_, String]`, so errors surface as one tidy line and the next
query runs.

## Files

| File          | What it is                                                        |
|---------------|-------------------------------------------------------------------|
| `table.fable` | `Val` (typed cell), CSV loader, aligned renderer                  |
| `query.fable` | tokenizer, recursive-descent parser, executor (filter/group/sort) |
| `main.fable`  | CLI glue, the showcase queries, and the golden `//? expect:` output |
| `cities.csv`  | the sample data set                                               |

Fable features on display: enums + exhaustive pattern matching for cells
and tokens, `Result` with `?` for error plumbing, a `Map` keyed by *enum
values* (structural hashing, insertion-order iteration) for `group by`,
generic `sort_by` comparators for `order by`, closures over module
functions, struct methods with in-place mutation for the parser cursor,
and multi-file modules with `pub` visibility. v0.6 additions in use:
tuple-destructuring loops (`for (key, bucket) in buckets.entries()`,
`for (c, cell) in cells.enumerate()`), bare `return Err(..)` / `break`
match arms on the parser's error paths, `index_of_from` to find the
lexer's closing quote, and `os.exit` in a value position (main.fable no
longer needs a dead `panic("unreachable")` after it). `Val.show`
deliberately skips v0.6's `to_fixed`, which would keep trailing zeros
("7.00") that csvql's tables drop ("7").
