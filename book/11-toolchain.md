# The Toolchain

The `fable` binary is more than a runner. The same checker that types your
program powers a language server; the same golden-test format this book uses
to verify its own snippets is a command you can point at your own code; and
a formatter and disassembler round out the set. None of it needs a plugin or
a second install — it is all in the one zero-dependency binary.

## `fable test`

Any `.fable` file with `//?` directives in its comments is a test. `//?
expect:` matches a line of stdout, `//? error:` asserts a compile error
containing a substring, and `//? panic:` a runtime panic:

```fable
fn square(x: Int) -> Int {
    x * x
}

println(square(9));            //? expect: 81
println([1, 2][5]);           //? panic: index 5, length 2
```

Run it with `fable test file.fable`, and `fable test dir/` walks a directory
recursively. A file with no directives passes by running silently, so a
whole example directory can be a test suite. The exit code is 1 on any
failure, so it drops into CI unchanged — Fable's own spec suite (294 tests)
and this book's snippets both run through this exact path.

## `fable fmt`

The formatter rewrites source into a canonical layout — comments and `//?`
directives preserved — and is line-width aware:

```sh
fable fmt src/main.fable            # print the formatted source
fable fmt -w src/*.fable            # rewrite files in place
fable fmt --width 60 src/main.fable # wrap at 60 columns instead of 100
```

It fits a construct on one line when it can and breaks it — arguments one
per line, chains before each `.`, binary operators before the operator —
when it can't, composing outermost-first. Because it works from the parsed
syntax and never from the original whitespace, `fmt` is idempotent:
formatting formatted code changes nothing.

## `fable build` — one program, one binary

`fable build` turns a whole program into a single self-contained executable.
Point it at a directory whose entry is `main.fable` (or at a `.fable` file
directly):

```sh
fable build demos/png -o png     # staple the program into ./png
./png                            # run it — no fable, no source tree needed
```

The binary carries everything the program touches: its imported modules, the
data files it reads, and the separate `.fable` files it hands to
`worker.spawn`. There is no code generation involved — the program's files
are *stapled* onto a copy of the interpreter (appended after its image, which
the binary finds by reading its own tail at startup), and unpacked into a
scratch directory the moment it runs. That is why the zero-dependency
promise survives: a stapled binary is just `fable` with a payload glued on.

The contract is simple to hold in your head: **a stapled binary behaves
exactly like `fable <the path you built>` run from the directory you built
it in.** Relative paths resolve the same way, workers spawn the same way,
output is byte-for-byte the same. Building `demos/png` and running the result
prints precisely what `fable demos/png/main.fable` prints.

One binary is per-target. To ship for machines you are not sitting at, build
the interpreter for another target and hand it to `build` as the launcher:

```sh
# staple the same program onto a launcher cross-compiled for another target
fable build demos/png --launcher ./fable-aarch64-macos -o png-macos
```

Because stapling is just byte concatenation, one host can assemble binaries
for every target it has a launcher for — which is exactly how the release
"demo zoo" is produced: each demo, cross-built for `x86_64` and `aarch64`
Linux and Windows, all from one runner. macOS is the one target that can't use
a plain append — a Mach-O with data past its `__LINKEDIT` fails code signing,
and Apple Silicon won't run an unsigned binary — so there the payload is
linked in as a `__DATA,__fablezoo` section instead, which `fable` reads back
out of its own image at startup.

## `fable lsp`

The checker already knows every type and every definition site; `fable lsp`
lets your editor ask. It speaks the Language Server Protocol over stdin and
stdout:

- **Diagnostics** as you type — the same checker as `fable check`, run over
  your unsaved buffer, imports and all.
- **Hover** — the checked type of the expression under the cursor.
- **Go to definition** — locals, globals, functions, and methods, across
  module files.
- **Completion** — methods, fields, and tuple indices after a `.` (answered
  from the last analysis that parsed, so it works mid-edit), module members
  after an import alias, the `math`/`fs`/`os` namespaces, and top-level
  names.

Point any LSP client at the binary. For editors with a generic client —
Neovim's built-in LSP, Helix — that is a few lines of config naming the
command and the file type:

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

The JSON-RPC is hand-rolled, so the server adds no dependency to the binary.

## `fable dis` and the rest

`fable dis file.fable` prints the compiled bytecode — the subject of the
previous chapter, and a good way to see what a piece of syntax actually
costs. `fable check` type-checks without running; `fable tokens` and `fable
ast` dump the earlier pipeline stages for debugging; and `fable repl` is an
incremental session with working imports and a `:type` query. The full CLI
is `fable` with no arguments, or `fable --help`.

---

Previous: [Under the Hood](10-under-the-hood.md) ·
Next: [Idioms and Style](12-idioms.md) ·
[Back to the index](README.md)
