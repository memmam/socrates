#!/usr/bin/env python3
"""
check_sos_vs_scipy.py — CI guard for the butter coefficient freeze
(ports/pyl/CONTRACT.md, "the per-filter freeze").

Reads sos_freeze.txt (each line `<design-id> b0 b1 b2 1 a1 a2`, design-id
of the form `butter,<order>,<btype>,<wn>[,<wn2>]` with exact round-trip
Wn), regenerates every design with REAL scipy
(`signal.butter(order, Wn, btype=..., output='sos')`), and compares every
coefficient.  A coefficient fails if

    |shim - scipy| > 1e-9 * max(|shim|, |scipy|)   and   |shim - scipy| > 1e-15

(the tiny absolute floor only forgives structural zeros).  Exits 0 iff
every design matches, 1 otherwise, with a per-design report.

Run in CI where scipy is installed:
    python3 check_sos_vs_scipy.py [path/to/sos_freeze.txt]
"""

import os
import sys

REL_TOL = 1e-9
ABS_FLOOR = 1e-15


def parse_freeze(path):
    designs = []  # list of (design_id, [rows])
    by_id = {}
    with open(path, 'r') as f:
        for lineno, line in enumerate(f, 1):
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            parts = line.split()
            if len(parts) != 7:
                raise ValueError('%s:%d: expected 7 fields, got %d'
                                 % (path, lineno, len(parts)))
            did = parts[0]
            row = [float(v) for v in parts[1:]]
            if did not in by_id:
                by_id[did] = []
                designs.append((did, by_id[did]))
            by_id[did].append(row)
    if not designs:
        raise ValueError('%s: no designs found' % path)
    return designs


def parse_design_id(did):
    parts = did.split(',')
    if len(parts) not in (4, 5) or parts[0] != 'butter':
        raise ValueError('unrecognized design id %r' % did)
    order = int(parts[1])
    btype = parts[2]
    wn = [float(v) for v in parts[3:]]
    if btype not in ('lowpass', 'highpass', 'bandpass'):
        raise ValueError('unrecognized btype in %r' % did)
    return order, btype, wn


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, 'sos_freeze.txt')

    try:
        from scipy import signal
    except ImportError:
        sys.stderr.write('FAIL: real scipy is required for this check '
                         '(run in CI with scipy installed)\n')
        return 1

    designs = parse_freeze(path)
    n_fail = 0
    worst_rel = 0.0
    worst_where = ''
    for did, rows in designs:
        order, btype, wn = parse_design_id(did)
        sos = signal.butter(order, wn if len(wn) > 1 else wn[0],
                            btype=btype, output='sos')
        ok = True
        why = ''
        if len(sos) != len(rows):
            ok = False
            why = 'section count %d != scipy %d' % (len(rows), len(sos))
        else:
            for si, (frozen, ref) in enumerate(zip(rows, sos)):
                for ci in range(6):
                    a = frozen[ci]
                    b = float(ref[ci])
                    diff = abs(a - b)
                    scale = max(abs(a), abs(b))
                    rel = diff / scale if scale > 0.0 else 0.0
                    if rel > worst_rel:
                        worst_rel = rel
                        worst_where = '%s section %d coeff %d' % (did, si, ci)
                    if diff > REL_TOL * scale and diff > ABS_FLOOR:
                        ok = False
                        why = ('section %d coeff %d: frozen %r vs scipy %r '
                               '(rel %.3g)' % (si, ci, a, b, rel))
                        break
                if not ok:
                    break
        if ok:
            print('PASS %s (%d sections)' % (did, len(rows)))
        else:
            print('FAIL %s: %s' % (did, why))
            n_fail += 1

    print('---')
    print('%d designs checked, %d failed; worst relative diff %.3g (%s)'
          % (len(designs), n_fail, worst_rel, worst_where or 'n/a'))
    return 1 if n_fail else 0


if __name__ == '__main__':
    sys.exit(main())
