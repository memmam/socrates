# pyl — the Python-to-Socrates translation layer: numerical contract

This file pins the semantics that BOTH implementations of the claudewave
port must follow exactly (claudewave is `pyl`'s only port so far; a second
`pyl` port would grow this contract, the same way `jsl`'s modules grow by
pull — see `ports/README.md`):

- `ports/claudewave/reference/pynp.py` — a **stdlib-only Python** shim
  implementing the numpy/scipy.signal subset below. The upstream
  claudewave `lib/*.py` files run **unmodified** against it (injected via
  `sys.modules['numpy'] = ...` etc.), producing ground-truth output.
- `ports/pyl/*.soc` — the Socrates layer implementing the same subset.

Where this contract and real scipy could disagree, CI (which can install
the real package) compares the shim's `butter`/SOS output against
`scipy.signal.butter(..., output='sos')` directly (the freeze-file check
below). The `pyl.nd` array/reduction functions (`geomspace`, `linspace`,
`cumsum`, `clip`, `take_lerp`, `gauss`, ...) have no equivalent real-numpy
cross-check anywhere — CI only verifies the Socrates port against this
shim, never the shim against real numpy — so for that surface, locally
and in CI alike, the shim is the executable contract. Parity between
shim-run upstream code
and the Socrates port is judged **numerically**: arithmetic-only paths are
expected bit-equal in f64; paths through libm transcendentals
(sin/cos/exp/tanh/pow) get a documented allowance of max abs diff
≤ 1e-9 per sample (report the actual max, always).

## Arrays (`pyl.nd`)

f64 throughout. An array is `{ ch: Int (1|2), data: List[Float] }`,
interleaved when stereo; `n = len(data)/ch`. Mono ops on stereo arrays
and vice versa are errors unless stated.

Constructors: `zeros(n)`, `zeros2(n)` (stereo), `ones(n)`, `full(n, v)`,
`arange(n)` (0..n-1 as floats), `linspace(a, b, n)` (numpy semantics:
endpoint included; n==1 → [a]; n==0 → empty; step = (b-a)/(n-1)),
`concat(parts)`, `stack2(l, r)` (two mono → stereo), `mono2(x)`
(duplicate mono to both channels), `geomspace(a, b, n)` (numpy
semantics: n log-spaced points, endpoints included).

Elementwise (same shape): `+ - * /`. Scalar: `adds(k)`, `muls(k)`,
`divs(k)`; `k_minus(k)` for `k - x`. Unary maps: `sinv`, `cosv`, `expv`,
`tanhv`, `absv`, `floorv`, `sqrtv`. All apply per component (stereo
included).

Reductions/scans (mono): `mean`, `max_abs`, `cumsum` (running sum, same
length, left to right, plain f64 accumulation).

`clip(lo, hi)` = per-component `min(max(x, lo), hi)`.
`where_lt(x, t, a, b)` = per-element `if x[i] < t { a[i] } else { b[i] }`.

Slicing (copies): `slice(from, to)` (frame indices, clamped, `to`
exclusive), `tail(k)` (last k frames), `pad_end(n)` (zero-pad to n
frames, truncate if longer). In-place: `set_range(at, src)` (copy src
frames in starting at frame `at`), `add_range(at, src, gain)`
(accumulate, clipped to the destination's end — matches
`out[si:ei] += chunk[:ei-si] * gain`).

`take_lerp(pos)` — pos is a mono array of fractional frame positions,
already clipped to `[0, n-2]` by the caller; result frame i =
`x[i0]*(1-frac) + x[min(i0+1, n-1)]*frac` with `i0 = trunc(pos[i])`,
`frac = pos[i] - i0`, per channel. (This is dsp.tape_wobble's fancy-index
expression as a primitive.)

`mul_env(env)` — stereo × mono broadcast (`y * env[:, None]`); on mono
it is plain `*`.

## Filters (`pyl.signal`)

Only what upstream uses — which a full call-site enumeration puts at:
Butterworth orders 2–4, btypes `low`, `band`, AND `high` (the hi-hat is
an order-3 highpass; the vocoder builds eighteen order-4 bandpasses over
`geomspace(80, 9000, 19)` band edges), as second-order sections, plus
`sosfilt`. An earlier revision of this contract claimed only orders 2–3
low/band; the freeze (33 designs) is the accurate inventory.

`butter(order, wn_low, wn_high, btype)` → `List[Sos]` where each `Sos`
is `{b0, b1, b2, a1, a2}` (a0 normalized to 1). Pinned algorithm:

1. Analog lowpass prototype poles (scipy `buttap` ordering):
   `p_k = -exp(i·π·(2k − N + 1)/(2N))` for k = 0..N-1 (unit circle,
   left half-plane; an earlier revision of this contract wrote
   `(2k+1)` which lands in the right half-plane for even N — caught
   during implementation); gain 1; no zeros.
2. Prewarp: `warped = 4·tan(π·Wn/2)` for each cutoff (fs = 2).
3. `low`: `p ← warped · p`, overall analog gain `warped^N`. `band`: with `bw = w2 - w1`,
   `w0 = sqrt(w1·w2)`: each prototype pole p becomes the pair
   `p' = (p·bw/2) ± sqrt((p·bw/2)² − w0²)` (2N poles), plus N zeros at
   s = 0; gain `bw^N`.
4. Bilinear (fs = 2): `z = (4 + s) / (4 − s)` for poles and zeros; each
   transform contributes gain `(4 − s)` to the denominator product —
   full digital gain = `analog_gain · Re(Π(4 − z_analog_zeros) / Π(4 −
   p_analog_poles))`; zeros at s = ∞ map to z = −1.
5. SOS pairing: scipy's `zpk2sos('nearest')`, which both sides
   implement outright (the shim ports it statement for statement, as
   does `ports/pyl/signal.soc` — its header records the steps):
   `_cplxreal` first (lexicographic (re, |im|) sort with 100·eps
   tolerance, run-sorting by |im| within equal-real runs, conjugate
   averaging), then sections filled **worst-pole-first** — the
   remaining pole nearest the unit circle seeds each section — from the
   LAST row to the first, with nearest-zero pairing, scipy's two
   odd-order special cases, and the overall gain folded into the FIRST
   emitted section's b row. For odd-order `low`, the leftover real pole
   forms a first-order section. Zeros: `low` → all digital zeros at
   z = −1, two per second-order section (the first-order section takes
   one); `band` → each second-order section takes one z = +1 and one
   z = −1 (`b ∝ [1, 0, −1]`). Section order and gain placement follow
   **the freeze file** (which follows scipy's `zpk2sos` output for
   these designs — note scipy places an odd order's real-pole section
   first and carries the overall gain in the first emitted section);
   the Socrates side must reproduce the freeze however it gets there.
6. **The per-filter freeze (authoritative):** prose descriptions of SOS
   pairing conventions are error-prone, so the binding artifact is a
   coefficient dump. The shim author implements steps 1–5, then writes
   the SOS matrix of **every distinct design the ported files construct**
   (all `signal.butter` call sites across synths/drums/dsp/ambience/
   vocoder — enumerate them from the source, including the order-3
   band-pass designs in `voice_whistle`, `render_crickets`, and
   `voice_shaker`, and the vocoder's per-band designs at its documented
   band edges) into
   `ports/claudewave/reference/sos_freeze.txt`, one line per section:
   `<design-id> b0 b1 b2 1 a1 a2` in shortest round-trip floats. The
   Socrates implementation must reproduce every frozen coefficient to
   ≤ 1e-12 relative. CI additionally regenerates the same designs with
   real scipy (`butter(..., output='sos')`) and fails if any frozen
   coefficient differs from scipy's by more than 1e-9 relative — the
   freeze cannot silently drift from the real library.

`sosfilt(sos, x)`: cascade in section order; each section is direct-form
II transposed with zero initial state:

```
y = b0·x + s1
s1 = b1·x − a1·y + s2
s2 = b2·x − a2·y
```

Stereo: filter channels independently (axis=0 semantics).

## Randomness (`pyl.rand`)

All randomness is a **pre-generated stream of unit floats** in a text
file (one float per line, shortest round-trip repr, generated once by
`ports/claudewave/reference/gen_stream.py` with Python's
`random.Random(20260714).random()`). Both sides consume the same file
with a cursor; running out of stream is a hard error.

Derived draws (pinned, identical both sides):
- `random()` = next unit float `u`.
- `uniform(a, b)` = `a + (b − a)·u`.
- `randint(a, b)` (inclusive) = `a + floor(u·(b − a + 1))`, clamped to b.
- `choice_pm1()` = `−1.0 if u < 0.5 else 1.0`.
- `gauss()` = Box–Muller: draw `u1, u2`;
  `sqrt(−2·ln(max(u1, 1e-300))) · cos(2π·u2)` (the sine twin is
  discarded). `randn(n)` = n successive `gauss()` calls.

The shim monkeypatches upstream's `random` and `np.random` to these.

## Audio I/O (`pyl.audio`) — the PAW format

Audio travels between the two implementations as PAW, a PPM-spirited
text format chosen for diffability. (The port initially surfaced a real
language gap here — Socrates had no binary file I/O — which became v0.7's
`Bytes` type. PAW remains the parity-comparison format, but the Socrates
side can also emit WAV directly now, via `audio.write_wav` over the
v0.8 `std.wav` module — see `docs/SPEC.md` § 7.1 — rather than only
through the Python-side `paw2wav.py` tool below.)

```
PAW1
<sr> <ch> <n>
<sample> [<sample>]     # one line per frame, ch floats per line
```

Floats are written in each side's shortest-round-trip form and compared
numerically, never textually. `ports/claudewave/tools/paw2wav.py`
(Python stdlib `wave` + `struct` only) converts to 16-bit PCM WAV for
listening; `wav2paw.py` converts back.

## Comparison (`ports/claudewave/reference/compare_paw.py`)

`python3 compare_paw.py truth.paw candidate.paw` → prints
`max_abs_diff=<float> frames=<n> ch=<c> allowed=<float>` and exits 0 iff
`max_abs_diff` is within `max(row, 2e-15)` for the battery item's row
in the script's per-item expected-max residual table (item name =
`truth.paw`'s basename; `0.0` for items measured bit-identical in the
reference environment, 2× the measured residual for the rest; the
2e-15 floor tolerates the oracle's own few-ulp drift across
numpy/libm environments) AND within the global 1e-9 outer bound. An
item with no table row is a comparison error. Shapes and sample rates
must match exactly (a mismatch on either is a hard comparison error,
independent of the residual check).
