# 9. The Toolchain Release

v0.4 is about everything around the language: a test runner, a standard
library, a language server, catchable panics, and lazy iterators that
needed no interpreter changes at all. As always, every snippet ran before
it was written down.

## 9.1 `fable test`

The golden-test format this book's interpreter has used all along — programs
with expectations in comments — is now a user-facing command. Any `.fable`
file with directives is a test:

```fable
// math_test.fable
fn square(x: Int) -> Int {
    x * x
}

println(square(9));            //? expect: 81
println([1, 2][5]);            //? panic: out of bounds
```

```text
$ fable test math_test.fable
ok: 1 test passed
```

`//? expect:` lines must match stdout in order; `//? error:` asserts a
compile error containing the substring; `//? panic:` a runtime panic. A file
with no directives passes by running silently. `fable test dir/` walks
recursively; the exit code is 1 on any failure, so it slots into CI as-is.
Fable's own 262-test spec suite runs through the identical code path.

## 9.2 The standard library

Five modules ship inside the binary — `import std.json;` works in any file,
with no install step and nothing on disk. The `std.` prefix is reserved.

```fable
import std.json;
import std.flags;

// Every step that can fail says so in its type; `?` threads them.
fn port_of(config: json.Json) -> Option[Float] {
    config.get("server")?.get("port")?.as_num()
}

match json.parse("\{\"server\": \{\"port\": 8080}}") {
    Ok(cfg) -> println(port_of(cfg)),      // Some(8080.0)
    Err(e) -> println("bad config: {e}"),
}

let args = os.args();
let verbose = flags.flag(args, "verbose");
let out = flags.value_or(args, "out", "build");
```

`std.json` is the whole JSON story (parse, stringify, pretty, and
`?`-friendly accessors); `std.flags` is deliberately rigid CLI parsing;
`std.path` and `std.strings` cover the textual chores. They're written in
Fable — readable under `std/` in the repository — and follow the same `pub`
rules as your code.

## 9.3 `fable lsp`

The checker has always known every type and every definition site; now your
editor can ask. `fable lsp` speaks the Language Server Protocol over stdio:

- **Diagnostics** as you type — the same loader/checker pipeline as
  `fable check`, run with your unsaved buffer overlaid, imports and all.
- **Hover** — the checked type of the expression under the cursor.
- **Go to definition** — locals, globals, functions, and methods, across
  module files.
- **Completion** (v0.5) — methods, fields, and tuple indices after a dot
  (answered from the last analysis that parsed, so it works mid-edit),
  module members after an import alias, `math`/`fs`/`os` namespaces, and
  top-level names.

Point any LSP client at the binary. For VS Code-compatible editors, that's
a config entry naming the command (`fable lsp`) and the file pattern
(`*.fable`); no plugin required for generic clients like Neovim's built-in
LSP or Helix:

```toml
# helix: languages.toml
[[language]]
name = "fable"
scope = "source.fable"
file-types = ["fable"]
language-servers = ["fable-lsp"]

[language-server.fable-lsp]
command = "fable"
args = ["lsp"]
```

The server is ~700 lines including its hand-rolled JSON — the zero-dependency
rule survived contact with JSON-RPC.

## 9.4 `try`: catchable panics

Panics abort the program — usually what you want, until you're fifty files
into a batch job. `try(f)` runs a function and turns a panic into data:

```fable
let results = ["10", "0", "x"].map(|s| try(|| 100 / s.parse_int().unwrap()));
println(results);
```

```text
[Ok(10), Err("division by zero"), Err("called `unwrap()` on `None`")]
```

The VM restores its stack completely — even a caught stack overflow leaves a
working machine. Two honest caveats: side effects before the panic persist
(`try` is a recovery boundary, not a transaction), and `os.exit` still ends
the process. It composes with `?`, so "run this risky thing, propagate
failure" is one line.

## 9.5 Lazy iterators — as a library

The most satisfying item in v0.4 is the one that needed zero interpreter
changes. `std.iter` builds lazy sequences out of nothing but structs and
closures:

```fable
import std.iter;

// An infinite sequence, filtered, truncated, realized.
println(iter.count_from(1).filter(|n| n % 3 == 0).take(4).collect());
// [3, 6, 9, 12]

// Nothing runs until a consumer pulls.
let touched = [];
let pipeline = iter.of([1, 2, 3, 4, 5]).map(|x| { touched.push(x); x * 10 });
println(touched);                    // []
println(pipeline.take(2).collect()); // [10, 20]
println(touched);                    // [1, 2]
```

An `Iter[T]` is a struct holding a `next: fn() -> Option[T]` closure; `map`
wraps it in another closure, `take` counts down in captured mutable state,
`collect` pulls until `None`. Closures with shared mutable upvalues, generic
methods, and first-class functions — chapter 3's machinery, assembled into a
feature other languages bake into their compilers.

## 9.6 Where the project stands

Four releases in: a statically-typed, garbage-collected language with
methods, modules, visibility, operator overloading, tail calls, catchable
panics, a standard library, a test runner, a REPL, a formatter, a
disassembler, and a language server — in dependency-free Rust, with every
feature specified in `docs/SPEC.md` and pinned by golden tests you can run
with the tool the tests themselves helped build.

---

Previous: [The Glue Chapter](08-glue.md) ·
Next: [The Field Test](10-field-test.md) ·
[Back to the index](README.md)
