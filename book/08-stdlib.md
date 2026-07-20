# The Standard Library and System Namespaces

Socrates ships with batteries of two kinds. **System namespaces** — `math`,
`fs`, `os`, the graphics namespaces `window` and `gfx` at the end of this
chapter, and the numeric namespaces of the next — are implemented in Rust,
always present, and used without an import, like `math.sin`. The
**standard library** — `std.json`, `std.set`, and friends — is written in
Socrates, compiled into the binary, and brought in with `import`. Nothing is on
disk to install either way.

## `fs` and `os`: touching the world

A program that only reads stdin and writes stdout can't do much glue work.
The `fs` and `os` namespaces cover the essentials, and everything fallible
returns `Result[_, String]`, so it composes with `?`:

```soc
// greeting.txt is tracked in the repo, so it picks one of a few presets
// each run — a stale copy left over from an earlier run reads differently
// from a fresh one, the same idea as a CLI's rotating startup tip.
let greetings = ["hello\nworld\n", "hi\nthere\n", "o hai\nmayfly\n", "beep\nboop\n"];
fs.write("greeting.txt", greetings[math.rand_int(0, greetings.len() - 1)]).unwrap();
let text = fs.read("greeting.txt").unwrap();
println(text.trim().split("\n").len());
println(fs.exists("greeting.txt"));

match os.run("echo", ["hi"]) {
    Ok(t) -> println("exit {t.0}: {t.1.trim()}"),
    Err(e) -> println("failed: {e}"),
}
```

```text
2
true
exit 0: hi
```

The full set: `fs.read`, `fs.write`, `fs.append`, `fs.exists`, `fs.is_dir`,
`fs.list_dir` (sorted), `fs.create_dir` (recursive), `fs.remove`, and
`fs.read_bytes`/`fs.write_bytes` for the `Bytes` of chapter 5; plus
`os.args()`, `os.env(name)`, `os.run(cmd, args)`, `os.exit(code)`, and
`os.time()`. A subprocess returns its whole story in one tuple: `Ok` means
the process *ran* — exit code `t.0`, stdout `t.1`, stderr `t.2` — while `Err`
means it could not be launched at all.

## The standard library

The `std.` prefix is reserved and needs no path. Each module is ordinary
Socrates, readable under `std/` in the repository, and follows the same `pub`
rules as your code. The most useful additions to what the builtins already
give you:

**A string builder.** Chapter 5 showed that growing a string with `+=` in a
loop is quadratic. `strings.Builder` wraps the collect-then-join pattern in
an object you push onto:

```soc
import std.strings;

let b = strings.builder();
b.push("Hello");
b.push(", world");
println("{b.build()} ({b.len()} chars)");
```

```text
Hello, world (12 chars)
```

`is_empty()` and `push_joined(sep, s)` round it out for line- or
field-oriented output — the latter pushes `sep` before every piece except
the first, so a CSV row or a joined list needs no manual `if len() > 0`:

```soc
import std.strings;

let row = strings.builder();
for field in ["a", "bb", "ccc"] {
    row.push_joined(",", field);
}
println(row.build());
```

```text
a,bb,ccc
```

**Collections beyond `List` and `Map`.** `std.set` is a `Set[T]` whose
`insert` returns whether the value was new — the one-call membership test:

```soc
import std.set;

let seen = set.new();
println(seen.insert(3));   // true: newly added
println(seen.insert(3));   // false: already present
seen.insert(1);
println(seen.to_list());   // insertion order
```

```text
true
false
[3, 1]
```

`std.deque` is a double-ended queue with `push_front`/`push_back` and
`pop_front`/`pop_back` (each an `Option`), and `std.lists` adds the
aggregates `List` leaves out — `sum`, `min`/`max` (returning `Option`),
`min_by`/`max_by`, `min_by_key`/`max_by_key`, and `fill`:

```soc
import std.lists;

println(lists.sum([1, 2, 3, 4]));
println(lists.max([3, 1, 4, 1, 5]));
println(lists.max_by_key(["a", "bbb", "cc"], |w| w.len()));
```

```text
10
Some(5)
Some("bbb")
```

**JSON**, parsed and generated, with `?`-friendly accessors:

```soc
import std.json;

match json.parse("\{\"port\": 8080}") {
    Ok(cfg) -> println(cfg.get("port").and_then(|p| p.as_num())),
    Err(e) -> println("bad config: {e}"),
}
```

```text
Some(8080.0)
```

(`\{` writes a literal brace, since a bare `{` in a Socrates string opens an
interpolation hole.) `json.stringify` and `json.pretty` go the other way.
Building a document by hand is just as direct with the constructors named
for what they build — `obj`, `arr`, `jstr`, `num`, `int`, `bool`, `null`
(`jstr`, not `str` — this module's own code needs the builtin `str()`):

```soc
import std.json;

let cfg = json.obj([("port", json.int(8080)), ("host", json.jstr("localhost"))]);
println(json.stringify(cfg));
```

```text
{"port":8080,"host":"localhost"}
```

**Lazy iterators**, built from nothing but structs and closures — no
interpreter support required. An `Iter[T]` computes nothing until a consumer
pulls:

```soc
import std.iter;

let touched = [];
let pipeline = iter.of([1, 2, 3, 4, 5]).map(|x| { touched.push(x); x * 10 });
println(touched);                     // [] — map hasn't run yet
println(pipeline.take(2).collect());  // [10, 20]
println(touched);                     // [1, 2] — only what was pulled
```

```text
[]
[10, 20]
[1, 2]
```

**Deferred, memoized computation.** A module-level `let table = build();`
already runs once, at import — but eagerly, whether or not `table` ends up
used. `Lazy[T]` defers the work to the first `get()` and caches it:

```soc
import std.lazy;

let mut calls = 0;
let table = lazy.of(|| { calls += 1; [1, 2, 3] });
println(calls);        // 0 — nothing has run yet
println(table.get());
println(table.get());  // cached — the thunk runs at most once
println(calls);
```

```text
0
[1, 2, 3]
[1, 2, 3]
1
```

**Vector and matrix math.** `std.glm` is 3D math named and shaped after
the GLM library — `vec3`, `perspective`, `look_at` — so graphics code
reads like graphics code everywhere else. `Vec2`/`Vec3`/`Vec4` carry the
operators plus `dot` and `length`; `normalize` and `lerp` are `Vec2`/`Vec3`
only (not `Vec4`), and `cross` is `Vec3`-only; `Mat4`
is column-major with the full constructor set (`translation`, `scaling`,
`rotation_*`, `perspective`, `ortho`, `look_at`), composed with `mul` and
applied with `mul_vec4`; `Quat` adds `from_axis_angle`, `slerp`, and
`to_mat4`. Pure Socrates, no native code:

```soc
import std.glm;

let a = glm.vec3(3.0, 0.0, 4.0);
println(a.length());
println(a.normalize().x);

let x = glm.vec3(1.0, 0.0, 0.0);
let y = glm.vec3(0.0, 1.0, 0.0);
let z = x.cross(y);
println("({z.x}, {z.y}, {z.z})");
println(x.dot(y));

let model = glm.rotation_y(math.pi / 2.0);
let view = glm.look_at(glm.vec3(0.0, 0.0, 3.0), glm.vec3(0.0, 0.0, 0.0), y);
let proj = glm.perspective(math.pi / 4.0, 16.0 / 9.0, 0.1, 10.0);
let mvp = proj.mul(view).mul(model);
let p = mvp.mul_vec4(glm.vec4(0.0, 0.0, 0.0, 1.0));
println(p.w);
```

```text
5.0
0.6
(0.0, 0.0, 1.0)
0.0
3.0
```

That last chain is the model-view-projection idiom exactly as a GL
tutorial writes it — `proj.mul(view).mul(model)` — and the `3.0` is the
origin sitting three units in front of the `look_at` eye, carried into
clip space in `w`.

Rounding out the set: `std.flags` is deliberately rigid CLI parsing
(`flag`, `value`, `value_or`, `positionals`), `std.path` handles the
textual path chores (`join`, `base_name`, `dir_name`, `ext`, `strip_ext`),
`std.wav` encodes RIFF/WAVE PCM audio over `Bytes` (mono or stereo,
16-bit — `demos/synthwave` builds one this way), `std.svg` builds SVG
documents (`demos/plot`'s charts), `std.markdown` converts Markdown to
HTML (`demos/mdsite`'s pages), and `std.crc`/`std.zlib`/`std.png`
together are a from-scratch PNG encoder (`demos/png`'s `out.png`).
The full method inventories live in the
[spec](../docs/SPEC.md); the point
of the standard library is that all of it is Socrates you can read, and none of
it cost the interpreter a line of Rust or the binary a dependency.

## `window` and `gfx`: native graphics

The `window` namespace opens a real OS window and `gfx` draws into it with
a GL-shaped call surface (programs, buffers, vertex arrays, uniforms,
textures, `draw_arrays`/`draw_elements`, `read_pixels`). Like `gpu` in the
next chapter, the backends are native raw-FFI code with zero Cargo
dependencies, behind cargo features only because they are platform code:
`window.create` is OpenGL on all three desktop platforms (X11/GLX,
Win32/WGL, Cocoa/CGL on Apple Silicon macOS only — `--features gl`),
`window.create_metal` is Metal on Apple Silicon macOS
(`--features metal`), and `window.create_vulkan` is Vulkan on
Linux/X11 and Windows (`--features vulkan`), where everything past
the platform's surface is one shared backend, so the two platforms behave
identically. Shader input follows the backend — GLSL source on OpenGL, MSL
on Metal, SPIR-V binaries on Vulkan — with `win.backend_name()` as the
branch point, and the `demos/glcube` demo renders the same golden frames
byte-for-byte on all three. A default build stays lean and reports itself
cleanly:

```soc
match window.create("socrates", 640, 480) {
    Ok(win) -> {
        win.make_current();
        gfx.clear(0.1, 0.2, 0.3, 1.0);
        win.swap_buffers();
        println("cleared one {win.backend_name()} frame");
        win.close();
    },
    Err(e) -> println("no window: {e}"),
}
```

```text
no window: windowing support not compiled in (build with --features gl)
```

(Build with `--features gl` on a machine with a display and the same
program prints `cleared one opengl frame` instead.) The full surface —
every `gfx` member, the per-backend shader conventions, and the
platform table — lives in the spec's § 7.3 and § 7.4.

---

Previous: [Programs Across Files](07-modules.md) ·
Next: [Workers, `fft`, and the GPU](09-workers.md) ·
[Back to the index](README.md)
