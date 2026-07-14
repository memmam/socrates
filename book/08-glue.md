# 8. The Glue Chapter

A glue language earns the name by touching the world: reading files, running
commands, taking arguments — and by letting you grow a personal toolbox of
modules that stay tidy. v0.3 is that release: `pub` visibility, operator
methods, a module search path, and the `fs`/`os` namespaces. Every snippet
here ran against the real interpreter.

## 8.1 `pub`: modules with actual boundaries

Module items are now **private by default**. `pub` exports a function, type,
top-level `let`, or an individual method:

```fable
// stack.fable
pub struct Stack {
    items: List[Int],
}

pub fn new() -> Stack {
    Stack { items: [] }
}

impl Stack {
    pub fn push(self, v: Int) {
        self.items.push(clamp(v));
    }

    pub fn pop(self) -> Option[Int] {
        self.items.pop()
    }

    fn size(self) -> Int {      // private: an internal helper
        self.items.len()
    }
}

fn clamp(v: Int) -> Int {       // private: importers never see it
    math.max(0, v)
}
```

Importers can call `stack.new()`, `s.push(3)`, and `s.pop()`; reaching for
`stack.clamp(..)` or `s.size()` is a compile error (E0339) naming the
private item and telling you where to add `pub`.

The rule that makes this predictable: **naming a foreign item requires
`pub`; using a value you hold does not.** If a public function hands you a
value of a private type, you can still pass it around, read its fields, and
match on its variants — you just can't write the type's name or call its
private methods. Inside a module (and in the root file, which nothing
imports), `pub` changes nothing.

## 8.2 Operator methods

Chapter 7's `Vec3` had `a.add(b).scale(2.0)`. Now the well-known method
names `add`, `sub`, `mul`, `div`, `rem`, and `neg` overload the operators
themselves:

```fable
struct V2 { x: Float, y: Float }

impl V2 {
    fn add(self, o: V2) -> V2 { V2 { x: self.x + o.x, y: self.y + o.y } }
    fn mul(self, k: Float) -> V2 { V2 { x: self.x * k, y: self.y * k } }
    fn neg(self) -> V2 { V2 { x: -self.x, y: -self.y } }
}

let a = V2 { x: 1.0, y: 2.0 };
let b = V2 { x: 10.0, y: 20.0 };
let c = a + b * 2.0;            // precedence as usual
println((-c).x);                // -21.0
```

Dispatch is on the *left* operand's type, so mixed signatures like
`vec * scalar` are natural — the parameter and return types are whatever
the method declares (`Money % 100` yielding an `Int` of change is fine).
Two deliberate refusals: `==` stays structural for every type, and `+=`
never dispatches — write `x = x + y`. If you use an operator on a type
without the method, the error tells you exactly what to define.

## 8.3 FABLE_PATH: a home for your toolbox

Imports resolve relative to the importing file first, then against each
directory in the colon-separated `FABLE_PATH` environment variable:

```sh
export FABLE_PATH="$HOME/fable-lib"
fable run anywhere/script.fable    # `import textutil;` finds ~/fable-lib/textutil.fable
```

A sibling file always wins over the search path, and the missing-module
error lists every location it tried. That's the whole feature — not a
package manager, just a place to keep the modules you reuse.

## 8.4 fs and os: touching the world

Until now a Fable program could read stdin and write stdout. The `fs` and
`os` namespaces (used like `math` — no import) cover the glue essentials.
Everything fallible returns `Result[_, String]`, so it composes with `?`:

```fable
fn head(path: String, n: Int) -> Result[String, String] {
    let text = fs.read(path)?;
    let lines = text.split("\n");
    let take = math.min(n, lines.len());
    let mut out = [];
    for i in 0..take {
        out.push(lines[i]);
    }
    Ok(out.join("\n"))
}

match head("Cargo.toml", 2) {
    Ok(s) -> println(s),
    Err(e) -> println("head: {e}"),
}
```

```text
[package]
name = "fable"
```

The full set: `fs.read`, `fs.write`, `fs.append`, `fs.exists`, `fs.is_dir`,
`fs.list_dir` (sorted), `fs.create_dir` (recursive), `fs.remove`; and
`os.args()`, `os.env(name)`, `os.run(cmd, args)`, `os.exit(code)`,
`os.time()`. Subprocesses return the whole story in one tuple:

```fable
match os.run("git", ["rev-parse", "--short", "HEAD"]) {
    Ok(t) -> println("rev {t.1.trim()} (exit {t.0})"),
    Err(e) -> println("not a git checkout: {e}"),
}
```

`Ok` means the process *ran* — its exit code is `t.0`, stdout `t.1`, stderr
`t.2`. `Err` means it couldn't be launched at all.

## 8.5 A real glue script

`examples/loc.fable` puts all of it together — a little line-counting tool:
`os.args()` for the target directory, a recursive `fs.list_dir`/`fs.is_dir`
walk, `fs.read` with `?` threading errors up to one `match` at the top,
methods on a `Tally` struct, and `+` overloaded to merge tallies. Run on
this repository:

```text
$ fable run examples/loc.fable .
ext        files    lines   blank  todos
.fable       257     5928     534      1
.md           12     4296     830      1
.rs           25    16104     818      0
.toml          1       20       4      0
total        295    26348    2186      2
```

Twenty-six thousand lines and two TODOs — one of which is this sentence,
which the tool dutifully counted the moment it was written. A line counter
measuring the chapter that describes it is a fitting place to leave the
tour.

---

Previous: [Methods, `?`, Modules, and Tail Calls](07-v02-features.md) ·
[Back to the index](README.md)
