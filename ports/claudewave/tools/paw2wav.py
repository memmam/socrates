#!/usr/bin/env python3
"""
paw2wav.py — convert a PAW text audio file (ports/pyl/CONTRACT.md) to a
16-bit PCM WAV for listening.  stdlib only (wave + struct).

Usage:
    python3 paw2wav.py in.paw out.wav

Samples are clamped to [-1, 1] and quantized as
round_half_away_from_zero-ish floor(x*32767 + 0.5); wav2paw.py divides by
32767, so a PAW -> WAV -> PAW round trip has max_abs_diff <= 0.5/32767
(< 1/32768) for in-range samples.
"""

import math
import struct
import sys
import wave


def read_paw(path):
    with open(path, 'r') as f:
        magic = f.readline().strip()
        if magic != 'PAW1':
            raise ValueError('%s: bad magic %r' % (path, magic))
        sr, ch, n = (int(v) for v in f.readline().split())
        if ch not in (1, 2):
            raise ValueError('%s: unsupported channel count %d' % (path, ch))
        data = []
        for i in range(n):
            parts = f.readline().split()
            if len(parts) != ch:
                raise ValueError('%s: frame %d has %d samples, expected %d'
                                 % (path, i, len(parts), ch))
            for p in parts:
                data.append(float(p))
    return sr, ch, n, data


def main():
    if len(sys.argv) != 3:
        sys.stderr.write('usage: python3 paw2wav.py in.paw out.wav\n')
        return 2
    sr, ch, n, data = read_paw(sys.argv[1])
    qs = []
    for v in data:
        if v > 1.0:
            v = 1.0
        elif v < -1.0:
            v = -1.0
        q = int(math.floor(v * 32767.0 + 0.5))
        if q > 32767:
            q = 32767
        elif q < -32768:
            q = -32768
        qs.append(q)
    with wave.open(sys.argv[2], 'wb') as w:
        w.setnchannels(ch)
        w.setsampwidth(2)
        w.setframerate(sr)
        w.writeframes(struct.pack('<%dh' % len(qs), *qs))
    print('wrote %s: %d frames, %d ch, %d Hz' % (sys.argv[2], n, ch, sr))
    return 0


if __name__ == '__main__':
    sys.exit(main())
