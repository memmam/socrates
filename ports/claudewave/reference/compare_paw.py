#!/usr/bin/env python3
"""
compare_paw.py — numeric comparison of two PAW files (ports/pyl/CONTRACT.md).

Usage:
    python3 compare_paw.py a.paw b.paw

Prints `max_abs_diff=<float> frames=<n> ch=<c>` and exits 0 iff
max_abs_diff <= 1e-9.  Shapes (frames, channels) must match exactly;
sample rates must match too (a mismatch is a comparison error, exit 2).
Floats are compared numerically, never textually.
"""

import sys


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
        sys.stderr.write('usage: python3 compare_paw.py a.paw b.paw\n')
        return 2
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
        if d > max_abs_diff:
            max_abs_diff = d
    print('max_abs_diff=%r frames=%d ch=%d' % (max_abs_diff, n_a, ch_a))
    return 0 if max_abs_diff <= 1e-9 else 1


if __name__ == '__main__':
    sys.exit(main())
