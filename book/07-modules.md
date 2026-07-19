# Programs Across Files

A program grows past one file eventually. Socrates's answer is modules: `import`
names another file, `pub` controls what that file exposes, and `SOCRATES_PATH`
gives your reusable modules a home. There is no package manager and no build
manifest — a Socrates program is still just files, related by their imports.

## `import`: one file naming another

`import a;` loads `a.soc` from the directory of the importing file and
binds it under the name `a`. Everything the module exposes is reached through
that name. Here is a two-file program — a geometry module and a `main` that
uses it:

```soc
// geo.soc
pub struct Point { x: Float, y: Float }

impl Point {
    pub fn dist(self, other: Point) -> Float {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

pub fn origin() -> Point {
    Point { x: 0.0, y: 0.0 }
}
```

```soc
// main.soc — run this one
import geo;

let p: geo.Point = geo.Point { x: 3.0, y: 4.0 };
println(p.dist(geo.origin()));
```

```text
5.0
```

Qualification with the module name works everywhere a name can appear:
function calls (`geo.origin()`), types (`geo.Point`), globals, variant
constructors (`geo.Shape.Circle(1.0)`), struct literals, and patterns. Two
things need no qualification at all — methods, which travel with their type
(so `p.dist(..)` works on a `geo.Point` anywhere), and variant patterns in a
`match` whose scrutinee type is already known, exactly like `Some`/`None`.

Nested paths and aliases round it out: `import a.b;` loads `a/b.soc` and
binds it as `b`, and `import a.b as m;` lets you pick the name.

## `pub`: modules with boundaries

Module items are **private by default**. `pub` exports a function, type,
top-level `let`, or individual method:

```soc
// counter.soc
pub struct Counter { n: Int }

pub fn new() -> Counter {
    Counter { n: 0 }
}

impl Counter {
    pub fn bump(self) -> Int {
        self.n = self.n + step();
        self.n
    }
}

fn step() -> Int {      // private: an internal helper, invisible to importers
    1
}
```

An importer can call the public surface:

```soc
import counter;

let c = counter.new();
println(c.bump());
println(c.bump());
```

```text
1
2
```

Reaching for the private helper is a compile error that names the item and
tells you where to add `pub`:

```soc errors
import counter;
println(counter.step());
```

```text
error[E0339]: function `counter.step` is private
  --> main.soc:2:17
   |
2 | println(counter.step());
   |                 ^^^^ not exported by its module
  note: add `pub` to `step` in the defining module
```

The rule that makes this predictable: **naming a foreign item requires
`pub`; using a value you already hold does not.** If a public function hands
you a value of a private type, you can still pass it around, read its fields,
and match on its variants — you just can't write the type's name or call its
private methods. Inside a module (and in the root file, which nothing
imports) `pub` changes nothing. A `pub` module global is readable from
outside but only its own module may assign it.

## Module semantics

The loading rules are deliberately simple:

- A module loads **once** per program, no matter how many files import it, so
  diamond-shaped import graphs share one copy of its state. Its top-level
  code runs once, before any importer's; the root file runs last.
- A circular import is a compile error with the cycle spelled out.
- Errors and panics point into the right file — stack traces span modules.

## `SOCRATES_PATH`: a home for your toolbox

Imports resolve relative to the importing file first, then against each
directory in the colon-separated `SOCRATES_PATH` environment variable:

```sh
export SOCRATES_PATH="$HOME/socrates-lib"
socrates run anywhere/script.soc    # `import textutil;` finds ~/socrates-lib/textutil.soc
```

A sibling file always wins over the search path, and the missing-module error
lists every location it tried. That is the whole feature — not a package
manager, just a place to keep the modules you reuse. The standard library
lives behind the reserved `std.` prefix and needs no path at all; it is the
subject of the next chapter.

---

Previous: [Error Handling](06-error-handling.md) ·
Next: [The Standard Library and System Namespaces](08-stdlib.md) ·
[Back to the index](README.md)
