#!/usr/bin/env python3
"""Cross-check Socrates's fft namespace against numpy.fft.

Usage: python3 tools/fft_crosscheck.py <path-to-socrates-binary>

Generates one Socrates program covering a spread of sizes (radix-2 and
Bluestein paths), runs it, and compares every bin against numpy at
1e-9 relative tolerance. The input signals use small integer-derived
values so both sides construct bit-identical f64 inputs.
"""

import subprocess
import sys
import tempfile
import os

import numpy as np

SIZES = [1, 2, 4, 8, 64, 256, 3, 5, 6, 12, 100, 384]
REL_TOL = 1e-9


def signal(n):
    re = [float((i * 37 + 11) % 17 - 8) + 0.25 for i in range(n)]
    im = [float((i * 23 + 5) % 13) * 0.5 for i in range(n)]
    return re, im


def socrates_list(xs):
    return "[" + ", ".join(repr(x) for x in xs) + "]"


def parse_list(line):
    line = line.strip()
    assert line.startswith("[") and line.endswith("]"), line
    body = line[1:-1].strip()
    return [float(p) for p in body.split(",")] if body else []


def main():
    socrates = sys.argv[1]
    prog = []
    for n in SIZES:
        re, im = signal(n)
        prog.append(f"let re{n} = {socrates_list(re)};")
        prog.append(f"let im{n} = {socrates_list(im)};")
        prog.append(f"let (fr{n}, fi{n}) = fft.fft(re{n}, im{n});")
        prog.append(f"println(fr{n});")
        prog.append(f"println(fi{n});")
        prog.append(f"let (br{n}, bi{n}) = fft.ifft(fr{n}, fi{n});")
        prog.append(f"println(br{n});")
        prog.append(f"println(bi{n});")
        prog.append(f"let (rr{n}, ri{n}) = fft.rfft(re{n});")
        prog.append(f"println(rr{n});")
        prog.append(f"println(ri{n});")

    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, "fft_check.soc")
        with open(path, "w") as f:
            f.write("\n".join(prog) + "\n")
        out = subprocess.run(
            [socrates, path], capture_output=True, text=True, check=True
        ).stdout.splitlines()

    lines = iter(out)
    worst = 0.0
    for n in SIZES:
        re, im = signal(n)
        x = np.array(re) + 1j * np.array(im)
        want_f = np.fft.fft(x)
        want_b = x  # ifft(fft(x)) round-trips to the input
        want_r = np.fft.rfft(np.array(re))
        for want, label in [(want_f, "fft"), (want_b, "ifft"), (want_r, "rfft")]:
            got = np.array(parse_list(next(lines))) + 1j * np.array(
                parse_list(next(lines))
            )
            assert got.shape == want.shape, f"n={n} {label}: shape {got.shape} vs {want.shape}"
            scale = max(np.abs(want).max(), 1.0)
            diff = np.abs(got - want).max() / scale
            worst = max(worst, diff)
            if diff > REL_TOL:
                print(f"FAIL n={n} {label}: rel diff {diff:.3e} > {REL_TOL}")
                sys.exit(1)
    print(f"fft numpy cross-check: {len(SIZES)} sizes ok, worst rel diff {worst:.3e}")


if __name__ == "__main__":
    main()
