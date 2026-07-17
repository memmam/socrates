# Workers, `fft`, and the GPU

The interpreter runs one program on one thread. Three namespaces reach past
that when a task is bigger than a single core's patience: `worker` runs
whole programs in parallel, `fft` drops to native code for the one numeric
kernel a tree-walker cannot make fast, and `gpu` (behind a build flag) hands
a compute shader to the graphics card. All three keep Fable's contract —
nothing shared implicitly, everything typed at the boundary.

## Workers: parallel isolates

`worker.spawn(file, args)` runs another Fable program on its own OS thread,
in its own VM with its own heap and garbage collector. The two sides share
*nothing*; they communicate only by passing `String` messages down a
channel. That is the whole concurrency model — no locks, because there is
nothing to lock.

A worker is an ordinary Fable file that checks `worker.is_worker()` and
talks to its parent with `worker.recv()` and `worker.send()`:

```fable
// square_worker.fable
if worker.is_worker() {
    while let Some(msg) = worker.recv() {
        let n = msg.parse_int().unwrap();
        worker.send(str(n * n));
    }
}
```

Guarding on `worker.is_worker()` means the file does nothing when run
directly — only when spawned does it enter the loop. `while let` is exactly
`while true { match worker.recv() { Some(msg) -> { .. }, None -> break } }`
(chapter 4) — the recv-loop is the idiom that motivated adding it. The
parent spawns the worker, sends work, and reads results off the handle:

```fable
let w = worker.spawn("square_worker.fable", []).unwrap();
w.send("3");
w.send("10");
println(w.recv().unwrap());
println(w.recv().unwrap());
println(w.join());
```

```text
9
100
Ok(())
```

`spawn` resolves the file relative to the entry script and returns a
`Result` — a missing file or a compile error in the worker comes back right
away, not as a surprise later. On the handle, `send` returns `false` once
the worker has finished, `recv` blocks until a message arrives (or returns
`None` when the worker is done and drained), and `join` waits for the worker
and reports how it ended. Inside the worker, `os.args()` returns the spawn
arguments.

`try_recv` is `recv`'s non-blocking twin, for a parent polling several
workers without picking one to block on: outer `None` means no message is
ready yet, `Some(None)` is `recv`'s own terminal state one level deeper (the
worker finished), and `Some(Some(s))` is a message:

```fable
let w = worker.spawn("square_worker.fable", []).unwrap();
println(w.try_recv());     // None — nothing sent yet, and it never blocks
w.send("6");
let mut got = None;
while got.is_none() {
    match w.try_recv() {
        Some(inner) -> { got = Some(inner); }
        None -> {}          // not ready — a real poller would check another worker
    }
}
println(got);
println(w.join());
```

```text
None
Some(Some("36"))
Ok(())
```

Two properties make workers pleasant to build on. First, **channels
buffer**: a parent can deal out every job up front and collect the answers
afterward, getting full parallelism with no synchronization code. Second,
**panics are isolated** — a worker that panics ends only its own thread, and
`join` returns the panic message as an `Err`, so the parent can log it and
carry on. The `parmandel` demo renders the Mandelbrot set across four
workers this way, and `swarm` builds a job-scheduling pool with panic
recovery; both pin their output exactly, because ordering is a matter of
protocol, not luck.

## `fft`: the native numeric kernel

A fast Fourier transform is the one piece of numeric work a tree-walking
interpreter cannot make competitive, so it is a builtin. The `fft` namespace
operates on split-complex signals — a real list and an imaginary list of the
same length — and follows numpy's conventions:

```fable
let (re, im) = fft.fft([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]);
println(re);   // the transform of an impulse is all ones
println(im);
```

```text
[1.0, 1.0, 1.0, 1.0]
[0.0, 0.0, 0.0, 0.0]
```

`fft.ifft` is the inverse (normalized by `1/n`), and `fft.rfft` takes a real
signal and returns the non-redundant half of the spectrum. Any length works
in O(n log n) — powers of two use an iterative radix-2 transform, everything
else routes through Bluestein's algorithm — and the results are
cross-checked against numpy in the test suite. The `spectra` demo builds a
chord analyzer on it; `synthwave` uses it to verify that the notes it
synthesized land on the frequencies it intended.

Reading a spectrum almost always means magnitude, not raw re/im — a
signal alternating every other sample puts all its energy in one bin.
The `magnitude` helper lives in `std.fft` (pure Fable — it is the
`sqrt(re²+im²)` one-liner, packaged), which also wraps `rfft` so the
`fft.` spellings survive the import:

```fable
import std.fft;

let (re, im) = fft.rfft([1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0]);
println(fft.magnitude(re, im));
```

```text
[0.0, 0.0, 4.0, 0.0, 0.0]
```

## `gpu`: compute shaders, behind a flag

The `gpu` namespace hands a compute kernel and a `Bytes` buffer to the
GPU and reads the result back. The backends are native raw-FFI code with
zero Cargo dependencies — Metal on Apple Silicon macOS (`--features
metal`); Vulkan, CUDA, and OpenCL on Linux/Windows (`--features vulkan` /
`cuda` / `opencl`); Direct3D 12 on Windows (`--features d3d12`) — so they
live behind cargo features only because they are platform code, not
because they pull anything in. A default build stays lean, and the
namespace still type-checks and runs — it just reports that it is
unavailable.

```fable
println(gpu.available());   // false in a default build; true with a native backend + device
```

Build with a backend feature and, on a machine with a usable device,
`gpu.run(shader, input, out_len, x, y, z)` compiles the kernel, uploads
the `Bytes`, dispatches the `x·y·z` index space, and returns the output
bytes as a `Result`. The kernel's dialect follows the backend —
`gpu.backend()` tells you which one is live: MSL source through `gpu.run`
on Metal, PTX on CUDA, HLSL on Direct3D 12, and SPIR-V binaries through
`gpu.run_spirv` on Vulkan and OpenCL (each in its own SPIR-V profile —
the spec's § 7.2 documents both):

```fable skip
let shader = "...MSL that doubles each f32...";
let input = bytes_of([/* four little-endian f32s */]);
match gpu.run(shader, input, 16, 4, 1, 1) {
    Ok(out) -> println(out.to_list()),
    Err(e) -> println("gpu: {e}"),
}
```

`docs/assets/metal_compute.fable`, `vulkan_compute.fable`,
`opencl_compute.fable`, `cuda_compute.fable`, and `d3d12_compute.fable`
are the runnable versions, one per backend. Fable
once took its single Cargo dependency here (wgpu, quarantined behind a
`gpu` feature); the native backends replaced it, and today every build of
Fable — any feature set — is the same zero-dependency language the rest
of this book describes.

---

Previous: [The Standard Library and System Namespaces](08-stdlib.md) ·
Next: [Under the Hood](10-under-the-hood.md) ·
[Back to the index](README.md)
