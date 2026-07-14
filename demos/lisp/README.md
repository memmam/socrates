# mini-lisp — a Lisp interpreter written in Fable

A small, classic tree-walking Lisp: an s-expression reader, an evaluator
with lexical closures, and a handful of builtins — about 400 lines of
Fable. The demo runs five sample Lisp programs from `programs/*.lisp`,
printing each program's source and every top-level result.

## Run it

From the repository root:

```sh
./target/release/fable demos/lisp/main.fable   # run the demo
./target/release/fable test demos/lisp         # golden tests (main + spec)
```

(`main.fable` also works with `demos/lisp/` as the working directory.)

## Sample output

```
;;; factorial.lisp
    ; factorial -- the classic recursion demo
    (define (fact n)
      (if (< n 2)
          1
          (* n (fact (- n 1)))))

    (fact 10)
    (fact 20)
=> 3628800
=> 2432902008176640000

...

;;; loop.lisp
    ...
    (sum-to 100000 0)
=> 5000050000

error demo: (car (quote ())) -> error: car of empty list
```

## The dialect

- **Atoms**: integers and symbols; `#t` / `#f` are the booleans, and only
  `#f` is falsy (Scheme truthiness).
- **Special forms**: `define` (with `(define (f a b) body)` sugar),
  `lambda` (lexical closures), `if`, `quote` (plus the `'x` reader sugar).
- **Builtins**: `+ - * / %` (variadic), chained comparisons
  `= < > <= >=`, and `cons car cdr list null? not`. Lists are proper
  lists only (no dotted pairs).
- **Comments** run from `;` to end of line.

## How it maps onto Fable

| Lisp concept | Fable feature |
|---|---|
| s-expressions | a recursive `enum Sexp { Num, Sym, Form(List[Sexp]) }` |
| runtime values | `enum Value`, with a `Closure` struct for lambdas |
| environments | `struct Env { vars: Map[String, Value], parent: Option[Env] }` — reference semantics make recursive `define` work |
| error reporting | `Result[Value, String]` + the `?` operator everywhere |
| overflow, stack overflow | `try()` converts runtime panics into Lisp-level errors |
| tail calls | Fable's TCO reaches *through* `eval`, so tail-recursive Lisp loops (see `programs/loop.lisp`) run in constant stack space |

Files:

- `reader.fable` — tokenizer + recursive-descent parser (`Sexp`)
- `eval.fable` — values, environments, special forms, builtins
- `main.fable` — driver: reads each `.lisp` file with `fs.read`, prints
  source and results; its full output is pinned by test directives
- `spec.fable` — 33 one-liner golden tests, happy paths and error paths
