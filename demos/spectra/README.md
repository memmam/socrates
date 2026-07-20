# spectra — a chord analyzer on `fft.rfft`

Chords are synthesized as sums of sinusoids and identified again purely
from their spectra. Two framing tricks make every pinned line exact:

1. **One-second windows.** Each signal is `n` samples at a nominal `n` Hz
   (`n = 1024`, and `n = 600` for the Bluestein case), so rfft bin `k` is
   exactly `k` Hz and an integer-frequency tone has **zero spectral
   leakage** — the whole magnitude `n/2` lands on one bin, the rest of the
   floor sits ~13 orders of magnitude down.
2. **5-limit just intonation on C4 = 240 Hz.** Just triads are exact
   integer frequency ratios, so gcd-reducing the peak bins gives the
   chord's shape as small integers: `4:5:6` major, `10:12:15` minor,
   `2:3` a bare fifth. Chord naming is a `Map` lookup, no logarithms.

For each of five chords (major, minor, power chord, an augmented shape
the table refuses to name, and a major triad at the non-power-of-two
length 600) the demo pins:

- an ASCII bar spectrogram (8-bin buckets, magnitude-labeled rows,
  Hz-labeled columns) assembled with `strings.Builder`
- the dominant bin found by `std.lists.max_by` under the `sort_by`
  comparator convention
- a hand-rolled top-k over bin powers (k linear scans, picked bins marked
  in a `std.set`; strict `>` keeps ties on the lower bin)
- the gcd-reduced ratio and the chord verdict
- three engine checks as Bools at 1e-9: Parseval
  (`sum x² == sum |X|²/n` via `fft.fft`), the `ifft(fft(x))` round trip,
  and `rfft == the first n/2+1 bins of fft`

`checks.soc` additionally cross-validates `fft.fft` against a naive
O(n²) DFT at `n = 12` (the Bluestein path), pins exact tiny transforms,
and unit-tests gcd/ratio/top-k/spectrogram on hand-built data.
`guardrails.soc` pins the fft argument contracts: two empty/mismatched-input
cases caught via `try()`, and one — `fft.ifft` on an empty spectrum — a
real `//? panic:` that ends the program.

## Run it

From the repository root:

```sh
./target/release/socrates demos/spectra/main.soc   # the full analysis
./target/release/socrates test demos/spectra         # golden tests
```

## Files

| File               | What it is                                                            |
|--------------------|-----------------------------------------------------------------------|
| `dsp.soc`        | synthesis (exact integer phase reduction), power spectrum, `by_power` comparator, `top_k`, gcd/ratio reduction, the Builder-assembled spectrogram |
| `main.soc`       | the five chord analyses, chord-quality and note-name tables, the three 1e-9 engine checks per chord |
| `checks.soc`     | unit goldens: gcd/ratios, exact single-tone spectrum, top-k tie-breaks, naive-DFT cross-check, tiny exact transforms, a 3-column toy spectrogram |
| `guardrails.soc` | pinned panics for the fft argument contracts                          |

## Determinism notes

Raw libm output is never pinned. `math.sin` feeds the signal, but every
golden line is either a derived integer (bin indices, gcd ratios, bar
heights — all separated from rounding boundaries by ~13 orders of
magnitude) or a `to_fixed` string of a value within ~1e-10 of an exact
decimal; every tolerance check prints a Bool at 1e-9. Bar heights and row
labels were chosen so no value sits near a `round()`/`to_fixed`
half-boundary (amplitudes 1.0/0.75/0.7/0.5 over 8 rows give heights
8/6/5.6/4 — nothing at `x.5`).

## v0.7 features on display

- `fft.rfft` as the analysis instrument; `fft.fft`/`fft.ifft` for
  Parseval and the round trip; the Bluestein path exercised at `n = 600`
  and `n = 12` and cross-checked against a naive DFT
- `strings.Builder` assembling the spectrogram (rows, axis, labels) in
  one O(n) pass
- `std.lists`: `max_by` (dominant bin), `max_float` (column peak, leak
  bounds), `fill` (zero imaginary parts, toy spectra)
- `std.set` marking picked bins inside `top_k`
