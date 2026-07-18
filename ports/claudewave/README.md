# claudewave DSP core, ported to Fable

A Fable port of the DSP core of **claudewave** by SkyeShark
([SkyeShark/claudewave](https://github.com/SkyeShark/claudewave)) — a
vaporwave / citypop remix toolkit whose `lib/` renders synth voices,
drums, tape/vinyl treatments, ambience layers and an 18-band channel
vocoder in numpy + scipy.signal. Upstream is MIT licensed (its README:
"License: MIT.") — see [`upstream/LICENSE-upstream`](upstream/LICENSE-upstream);
the six ported source files are vendored byte-for-byte in
[`upstream/`](upstream/) so CI can rerun them without a network fetch.

This is the first port through the [`pyl` translation layer](../pyl/)
(numpy / scipy.signal / random / audio subset — the numerical contract is
[`ports/pyl/CONTRACT.md`](../pyl/CONTRACT.md)). Each `.fable` file is
structured function-for-function against its upstream `.py` so the two
read side by side:

| Fable | Upstream | Contents |
|-------|----------|----------|
| `synths.fable` | `upstream/synths.py` | ADSR, rhodes/sub/bell/juno/FM-lead/slap-bass/whistle voices, chord helpers |
| `drums.fable` | `upstream/drums.py` | kick, brush snare, snap, shaker, hat, conga, beat-grid renderer |
| `dsp.fable` | `upstream/dsp.py` | RMS normalize, tape wobble, sidechain pump, vinyl crackle, hiss, fades, limiter |
| `choppers.fable` | `upstream/choppers.py` | hook extraction + echoing loop placement |
| `ambience.fable` | `upstream/ambience.py` | water, crickets, wind, equal-power stereo pan |
| `vocoder.fable` | `upstream/vocoder.py` | chord carrier, 18-band channel vocoder, ring mod |

## Run it

```sh
cargo build --release

# 1. generate the shared random stream (a runtime artifact — never commit it)
python3 ports/claudewave/reference/gen_stream.py /tmp/cw/rand_stream.txt

# 2. ground truth: the UNMODIFIED upstream .py files run against the
#    stdlib-only pynp shim (sys.modules injection; see run_upstream.py)
CLAUDEWAVE_STREAM=/tmp/cw/rand_stream.txt \
    python3 ports/claudewave/reference/run_upstream.py /tmp/cw/truth

# 3. the same 32-item battery from the Fable port
FABLE_PATH=ports CLAUDEWAVE_STREAM=/tmp/cw/rand_stream.txt \
    ./target/release/fable ports/claudewave/battery.fable /tmp/cw/fable

# 4. numeric comparison, item by item (exit 0 iff each item's max abs
#    diff is within its row in compare_paw.py's expected-max table,
#    1e-9 as the global outer bound)
for p in /tmp/cw/truth/*.paw; do
  python3 ports/claudewave/reference/compare_paw.py "$p" "/tmp/cw/fable/$(basename "$p")"
done
diff /tmp/cw/truth/chords.txt /tmp/cw/fable/chords.txt
```

Audio travels as PAW, the contract's diffable text format;
`tools/paw2wav.py` converts to 16-bit WAV for listening. Battery item i
seeks the shared stream cursor to offset i·100000 before rendering, so
every item is independently reproducible on both sides;
`battery.fable OUTDIR ITEM` re-renders a single item.

## The receipts

All randomness is a pre-generated unit-float stream consumed identically
by both sides; Butterworth designs are pinned by the 33-design
coefficient freeze (`reference/sos_freeze.txt`, cross-checked against
real scipy in CI). Parity is judged numerically, per item:
`compare_paw.py` carries a per-item expected-max residual table, and
each item must stay within **its own row** (with a global 1e-9 outer
bound on every item).

Enforced by CI on every push — **32/32 items pass their rows**. 29 rows
are `0.0`: those items must be **bit-identical** (`max_abs_diff=0.0`),
so a bit-exact item can never silently degrade. The three remaining
rows are 2× the measured residual — the residues sit at the f64
rounding floor, five-plus orders of magnitude under the 1e-9 outer
bound:

| item | measured max abs diff | enforced row | source of the residue |
|------|-----------------------|--------------|-----------------------|
| `voice_slap_bass_110` | 1.39e-16 | 2.8e-16 | layer `tanh` (exp-based formula vs libm tanh) |
| `voice_whistle_880` | 1.11e-16 | 2.3e-16 | order-3 bandpass SOS coefficients (≤ 1.78e-16 relative vs shim) |
| `amb_crickets` | 2.08e-17 | 4.2e-17 | order-3 bandpass SOS coefficients (same freeze residue) |

`chords.txt` (the chord-helper ground truth: parse/root/voicing/pad plus
`midi_to_hz` reprs) is byte-identical between the two sides.

## Scope notes

- Ported: everything in the six files above. Out of scope (declared in
  the port contract): upstream `analysis.py`, `viz.py`, `ace_step.py`,
  and the two `dsp.py` functions that need `soundfile` /
  `scipy.signal.resample` — `load_stereo` and `resample_rate`.
- Fable has no default arguments: call sites pass the upstream defaults
  explicitly (each function's doc comment records them). Tuple
  parameters (`noise_band`, `chirp_freq_band`) are flattened to two
  floats.
- Upstream reads module-global `random`; the port mirrors the harness's
  `sys.modules['random']` injection with a per-module `set_stream()`
  taking the shared `pyl.rand.Stream` (one object, one cursor).
- A Python `None` chord name maps to any unparseable name (e.g. `""`):
  `parse_chord` returns `Option`, `chord_voicing_midi` returns `[]`,
  and callers skip the bar exactly like upstream.
- The port added three helpers to `pyl.nd` (pinned in
  `ports/pyl/spec.fable`): `channel(c)` (`x[:, c]`), `mean_all()`
  (`np.mean` over every component) and `max_abs_all()`
  (`np.max(np.abs(x))` on any shape).
