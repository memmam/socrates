#!/usr/bin/env python3
"""
gen_stream.py — generate rand_stream.txt per ports/pyl/CONTRACT.md.

The stream is the single source of randomness for both the shim-run
upstream code and the Socrates port: unit floats from Python's
random.Random(20260714).random(), one per line, shortest round-trip repr.

Usage:
    python3 gen_stream.py [OUTPATH] [COUNT]

Defaults: OUTPATH = rand_stream.txt next to this script, COUNT = 4000000
(the battery's highest item offset is ~3.1M and no item consumes more
than its 100000-float stride, so 4M leaves ample headroom; running out
of stream is a hard error by design).

The stream file is a runtime artifact: generate it, do NOT commit it.
"""

import os
import random
import sys


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    out = sys.argv[1] if len(sys.argv) > 1 else os.path.join(here, 'rand_stream.txt')
    count = int(sys.argv[2]) if len(sys.argv) > 2 else 4_000_000
    rng = random.Random(20260714)
    chunk = []
    with open(out, 'w') as f:
        for i in range(count):
            chunk.append(repr(rng.random()))
            if len(chunk) == 100_000:
                f.write('\n'.join(chunk))
                f.write('\n')
                chunk = []
        if chunk:
            f.write('\n'.join(chunk))
            f.write('\n')
    print('wrote %d floats to %s' % (count, out))
    return 0


if __name__ == '__main__':
    sys.exit(main())
