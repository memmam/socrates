#!/usr/bin/env python3
"""Interleaved A/B benchmark between two Fable checkouts.

Usage: bench/ab.py BASE_DIR HEAD_DIR [--micro-n N] [--macro-n N]
                   [--targets name,name,...] [--threshold PCT]

Each side is a full checkout with its own `target/release/fable[.exe]`
already built; every target runs against *its own* tree, so the two sides
stay fair even when bench/demo/test sources differ between the refs
(a binary is never asked to run the other ref's programs).

Method (bench/RESULTS.md): runs are interleaved A,B,A,B,... within one
batch so machine drift hits both sides equally; the minimum wall time per
side is the least-noise estimator; only the relative delta is meaningful.
Every bench program prints a checksum, so a wrong-answer "optimization"
fails the run instead of winning it.

The exit code is 0 unless a target crashes; judging deltas against the
noise threshold is the caller's job (the table marks |delta| >= the
threshold, default 3%).
"""
import argparse
import os
import subprocess
import sys
import time

MICRO_DIR = "bench"
MACROS = [
    ("lisp", ["demos/lisp/main.fable"]),
    ("checkers", ["demos/checkers/main.fable"]),
    ("wfc", ["demos/wfc/main.fable"]),
    ("sudoku", ["demos/sudoku/main.fable"]),
    ("reversi", ["demos/reversi/main.fable"]),
    ("spectra", ["demos/spectra/main.fable"]),
    ("png", ["demos/png/main.fable"]),
]
# The spec suite is deliberately not a target: its sources move with each
# ref, so cross-ref wall times compare different suites, not the binary.


def binary(root):
    # Absolute, because the subprocess runs with cwd=root: POSIX exec
    # resolves a relative program path in the *child* cwd (root/root/...),
    # while Windows resolves it in the parent's — absolute is the only
    # spelling that means the same binary on both.
    exe = os.path.join(os.path.abspath(root), "target", "release", "fable")
    if os.name == "nt" or not os.path.exists(exe):
        win = exe + ".exe"
        if os.path.exists(win):
            return win
    return exe


def targets_for(root, filter_names):
    out = []
    micro = os.path.join(root, MICRO_DIR)
    for f in sorted(os.listdir(micro)):
        if f.endswith(".fable"):
            name = f[: -len(".fable")]
            out.append((name, [os.path.join(MICRO_DIR, f)], "micro"))
    for name, args in MACROS:
        out.append((name, args, "macro"))
    if filter_names:
        keep = set(filter_names)
        out = [t for t in out if t[0] in keep]
    return out


def run_once(root, args):
    t0 = time.perf_counter()
    subprocess.run(
        [binary(root)] + args,
        cwd=root,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        check=True,
    )
    return time.perf_counter() - t0


def main():
    # Windows Python pipes stdout as cp1252, which cannot encode the ⚠
    # marker; the table is UTF-8 markdown wherever it lands.
    sys.stdout.reconfigure(encoding="utf-8")
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("head")
    ap.add_argument("--micro-n", type=int, default=5)
    ap.add_argument("--macro-n", type=int, default=3)
    ap.add_argument("--targets", default="")
    ap.add_argument("--threshold", type=float, default=3.0)
    opts = ap.parse_args()

    for side in ("base", "head"):
        exe = binary(getattr(opts, side))
        if not os.path.exists(exe):
            sys.exit(f"{side} binary missing: {exe} (build both sides first)")

    filt = [t for t in opts.targets.split(",") if t]
    rows = []
    for name, args, kind in targets_for(opts.head, filt):
        # A bench that exists on only one ref (added/removed between the
        # two) cannot be compared — report the row and move on. Checked
        # against the source files, never inferred from a run failure: a
        # failed run is a real failure and must fail the job.
        if not all(
            os.path.exists(os.path.join(root, a))
            for root in (opts.base, opts.head)
            for a in args
        ):
            rows.append((name, None, None, None))
            continue
        n = opts.micro_n if kind == "micro" else opts.macro_n
        try:
            # One unmeasured warm-up per side, then strict interleaving.
            run_once(opts.base, args)
            run_once(opts.head, args)
            bs, hs = [], []
            for _ in range(n):
                bs.append(run_once(opts.base, args))
                hs.append(run_once(opts.head, args))
        except subprocess.CalledProcessError:
            print(f"{name}: FAILED to run on one side", file=sys.stderr)
            sys.exit(1)
        b, h = min(bs), min(hs)
        rows.append((name, b, h, 100.0 * (h - b) / b))

    print(f"| target | base (s) | head (s) | delta |")
    print(f"|--------|---------:|---------:|------:|")
    worst = 0.0
    for name, b, h, d in rows:
        if b is None:
            print(f"| {name} | — | — | (only on one ref) |")
            continue
        mark = " ⚠" if abs(d) >= opts.threshold else ""
        print(f"| {name} | {b:.4f} | {h:.4f} | {d:+.1f}%{mark} |")
        worst = max(worst, d)
    print()
    print(
        f"best-of interleaved (micro n={opts.micro_n}, macro n={opts.macro_n}); "
        f"⚠ marks |delta| >= {opts.threshold}% (the noise floor on shared "
        f"runners — judge marked rows, ignore the rest)."
    )


if __name__ == "__main__":
    main()
