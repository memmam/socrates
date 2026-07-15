#!/usr/bin/env python3
"""
wav2paw.py — convert a 16-bit PCM WAV back to the PAW text audio format
(ports/pyl/CONTRACT.md).  stdlib only (wave + struct).

Usage:
    python3 wav2paw.py in.wav out.paw

Integer samples are mapped to floats by division by 32767 (the inverse of
paw2wav.py's quantizer), written in shortest round-trip repr.
"""

import struct
import sys
import wave


def main():
    if len(sys.argv) != 3:
        sys.stderr.write('usage: python3 wav2paw.py in.wav out.paw\n')
        return 2
    with wave.open(sys.argv[1], 'rb') as w:
        ch = w.getnchannels()
        width = w.getsampwidth()
        sr = w.getframerate()
        n = w.getnframes()
        if width != 2:
            raise ValueError('only 16-bit PCM WAV is supported (got %d bytes)'
                             % width)
        if ch not in (1, 2):
            raise ValueError('only 1 or 2 channels supported (got %d)' % ch)
        raw = w.readframes(n)
    qs = struct.unpack('<%dh' % (n * ch), raw)
    lines = ['PAW1', '%d %d %d' % (sr, ch, n)]
    if ch == 1:
        for q in qs:
            lines.append(repr(q / 32767.0))
    else:
        for i in range(n):
            lines.append(repr(qs[2 * i] / 32767.0) + ' '
                         + repr(qs[2 * i + 1] / 32767.0))
    with open(sys.argv[2], 'w') as f:
        f.write('\n'.join(lines))
        f.write('\n')
    print('wrote %s: %d frames, %d ch, %d Hz' % (sys.argv[2], n, ch, sr))
    return 0


if __name__ == '__main__':
    sys.exit(main())
