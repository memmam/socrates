# mini-lisp — a Lisp interpreter written in Socrates

A small, classic tree-walking Lisp: an s-expression reader, an evaluator
with lexical closures, and a handful of builtins — about 740 lines of
Socrates. The demo runs six sample Lisp programs from `programs/*.lisp`,
printing each program's source and every top-level result.

## Run it

From the repository root:

```sh
./target/release/socrates demos/lisp/main.soc   # run the demo
./target/release/socrates test demos/lisp         # golden tests (main + spec)
```

(`main.soc` also works with `demos/lisp/` as the working directory.)

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
  `lambda` (lexical closures), `if`, `let` (parallel bindings, like
  Scheme's), `quote` (plus the `'x` reader sugar). Reserved words cannot
  be `define`d; duplicate `lambda` parameters and duplicate `let`
  bindings are errors.
- **Builtins**: `+ - * / %` (variadic), chained comparisons
  `= < > <= >=`, and `cons car cdr list null? not`. Lists are proper
  lists only (no dotted pairs).
- **Comments** run from `;` to end of line.

## How it maps onto Socrates

| Lisp concept | Socrates feature |
|---|---|
| s-expressions | a recursive `enum Sexp { Num, Sym, Form(List[Sexp]) }` |
| runtime values | `enum Value`, with a `Closure` struct for lambdas |
| environments | `struct Env { vars: Map[String, Value], parent: Option[Env], specials: set.Set[String] }` — reference semantics make recursive `define` work |
| special-form dispatch | a `std.set` `Set[String]` of reserved words, built once in `global_env` and shared by reference with every child scope (a pointer copy per scope, not a set copy); one probe decides special-vs-application, and the same set rejects `(define if 3)` |
| printing | `strings.Builder` threaded through the recursive printers (`show` in both `eval` and `reader`) — O(output) instead of re-copying child strings at every nesting level |
| duplicate names | `Set.insert -> Bool` ("did it change?") catches `(lambda (x x) ...)` and `(let ((a 1) (a 2)) ...)` |
| error reporting | `Result[Value, String]` + the `?` operator everywhere |
| overflow, stack overflow | `try()` converts runtime panics into Lisp-level errors |
| tail calls | Socrates's TCO reaches *through* `eval`, so tail-recursive Lisp loops (see `programs/loop.lisp`) run in constant stack space |

Files:

- `reader.soc` — tokenizer + recursive-descent parser (`Sexp`)
- `eval.soc` — values, environments, special forms, builtins
- `main.soc` — driver: reads each `.lisp` file with `fs.read`, prints
  source and results; its full output is pinned by test directives
- `spec.soc` — 41 one-liner golden tests, happy paths and error paths
