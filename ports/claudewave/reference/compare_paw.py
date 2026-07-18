#!/usr/bin/env python3
"""
compare_paw.py — numeric comparison of two PAW files (ports/pyl/CONTRACT.md).

Usage:
    python3 compare_paw.py truth.paw candidate.paw

Prints `max_abs_diff=<float> frames=<n> ch=<c> allowed=<float>` and exits
0 iff max_abs_diff <= max(the battery item's row in EXPECTED_MAX below,
ULP_FLOOR) AND <= the global 1e-9 outer bound.  The item name is the
first argument's basename without `.paw`; a name with no row is a
comparison error (exit 2) — every battery item must have an explicit
expected-max residual, so an item cannot silently degrade to "still
under 1e-9".

Rows are the RECORD of what was measured in the reference environment
(python 3.11 / numpy 2.4 / scipy 1.17, 2026-07-18): 0.0 for the 29
items measured bit-identical there, and 2x the measured residual for
the three items whose divergence sits at the f64 rounding floor.
Enforcement adds ULP_FLOOR because the upstream oracle's own output
drifts by a few ulps across environments — the recorded instance:
dsp_rms_normalize measured 0.0 in the reference environment and
6.7e-16 on the ubuntu-latest CI runner's numpy the same day (the Socrates
side is deterministic, so upstream(CI) != upstream(local)).  The floor
still fails anything algorithmic (a real degradation lands orders of
magnitude above 2e-15); only oracle-side libm/numpy rounding hides
under it.

    voice_slap_bass_110   1.3877787807814457e-16  (layer tanh vs libm tanh)
    voice_whistle_880     1.1102230246251565e-16  (order-3 bandpass SOS)
    amb_crickets          2.0816681711721685e-17  (same freeze residue)

Shapes (frames, channels) must match exactly; sample rates must match
too (a mismatch is a comparison error, exit 2).  Floats are compared
numerically, never textually; a NaN difference is an automatic failure.
"""

import math
import os
import sys

# Per-item expected-max residual (the enforced table). 0.0 = bit-identical.
EXPECTED_MAX = {
    'env_adsr': 0.0,
    'voice_rhodes_220': 0.0,
    'voice_sub_220': 0.0,
    'voice_bell_220': 0.0,
    'voice_fm_lead_220': 0.0,
    'voice_slap_bass_110': 2.8e-16,
    'voice_juno_pad_220': 0.0,
    'voice_whistle_880': 2.3e-16,
    'drum_kick': 0.0,
    'drum_brush_snare': 0.0,
    'drum_snap': 0.0,
    'drum_shaker': 0.0,
    'drum_hat': 0.0,
    'drum_conga': 0.0,
    'drums_on_beats': 0.0,
    'dsp_rms_normalize': 0.0,
    'dsp_tape_wobble': 0.0,
    'dsp_sidechain_pump': 0.0,
    'dsp_vinyl_crackle': 0.0,
    'dsp_cassette_hiss': 0.0,
    'dsp_pad_to': 0.0,
    'dsp_mix_fades': 0.0,
    'dsp_limit_peak': 0.0,
    'chop_extract_hook': 0.0,
    'chop_place_hook_loops': 0.0,
    'amb_water': 0.0,
    'amb_crickets': 4.2e-17,
    'amb_wind': 0.0,
    'amb_stereo_pan': 0.0,
    'vocoder_carrier': 0.0,
    'vocoder_channel': 0.0,
    'vocoder_ring_mod': 0.0,
}

GLOBAL_BOUND = 1e-9  # the outer bound; every row above is far under it

# Oracle-environment drift tolerance (see the module docstring): the
# enforced bound per item is max(row, ULP_FLOOR). A few ulps at unit
# scale — algorithmic drift lands far above it.
ULP_FLOOR = 2e-15


def read_paw(path):
    with open(path, 'r') as f:
        magic = f.readline().strip()
        if magic != 'PAW1':
            raise ValueError('%s: bad magic %r' % (path, magic))
        header = f.readline().split()
        if len(header) != 3:
            raise ValueError('%s: bad header' % path)
        sr, ch, n = (int(v) for v in header)
        if ch not in (1, 2) or n < 0 or sr <= 0:
            raise ValueError('%s: bad header values sr=%d ch=%d n=%d'
                             % (path, sr, ch, n))
        data = []
        for i in range(n):
            parts = f.readline().split()
            if len(parts) != ch:
                raise ValueError('%s: frame %d has %d samples, expected %d'
                                 % (path, i, len(parts), ch))
            for p in parts:
                data.append(float(p))
        rest = f.read().strip()
        if rest:
            raise ValueError('%s: trailing data after %d frames' % (path, n))
    return sr, ch, n, data


def main():
    if len(sys.argv) != 3:
        sys.stderr.write('usage: python3 compare_paw.py truth.paw candidate.paw\n')
        return 2
    item = os.path.basename(sys.argv[1])
    if item.endswith('.paw'):
        item = item[:-len('.paw')]
    if item not in EXPECTED_MAX:
        sys.stderr.write('error: no EXPECTED_MAX row for battery item %r '
                         '(add one to compare_paw.py)\n' % item)
        return 2
    allowed = max(EXPECTED_MAX[item], ULP_FLOOR)
    try:
        sr_a, ch_a, n_a, da = read_paw(sys.argv[1])
        sr_b, ch_b, n_b, db = read_paw(sys.argv[2])
    except (OSError, ValueError) as e:
        sys.stderr.write('error: %s\n' % e)
        return 2
    if (ch_a, n_a) != (ch_b, n_b):
        sys.stderr.write('shape mismatch: %d frames x %d ch vs %d frames x %d ch\n'
                         % (n_a, ch_a, n_b, ch_b))
        return 2
    if sr_a != sr_b:
        sys.stderr.write('sample rate mismatch: %d vs %d\n' % (sr_a, sr_b))
        return 2
    max_abs_diff = 0.0
    for x, y in zip(da, db):
        d = abs(x - y)
        if math.isnan(d):
            max_abs_diff = float('inf')
            break
        if d > max_abs_diff:
            max_abs_diff = d
    print('max_abs_diff=%r frames=%d ch=%d allowed=%r'
          % (max_abs_diff, n_a, ch_a, allowed))
    if max_abs_diff > allowed:
        sys.stderr.write('%s: residual %r exceeds its expected max %r\n'
                         % (item, max_abs_diff, allowed))
        return 1
    return 0 if max_abs_diff <= GLOBAL_BOUND else 1


if __name__ == '__main__':
    sys.exit(main())
