#!/usr/bin/env bash
# bench/run.sh — single-binary sequential profiling convenience.
#
# Times each micro bench (bench/*.soc) and each macro target (heavy
# demo mains + the spec suite) N times against ./target/release/socrates and
# reports the MINIMUM wall time (least-noise estimator).
# Usage: bench/run.sh [N]. The binary must already be built:
# cargo build --release
#
# This is NOT the A/B gate. Perf claims are judged by the interleaved
# cross-binary comparison: `bench/ab.py BASE_DIR HEAD_DIR` locally, and
# the four-architecture Bench A/B workflow
# (.github/workflows/bench.yml, fired by pushing a `bench/<name>`
# branch) for the matrix verdict. Use run.sh to see where one binary
# spends its time, not to compare two.
#
# A failing target fails the script (exit 1).
set -u
N="${1:-3}"
BIN=./target/release/socrates
command -v python3 >/dev/null || { echo "needs python3"; exit 1; }
STATUS=0

run_target() {
    local name="$1"; shift
    local best=""
    for _ in $(seq "$N"); do
        local t
        t=$(python3 - "$@" <<'PY'
import subprocess, sys, time
t0 = time.perf_counter()
subprocess.run(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
print(f"{time.perf_counter() - t0:.4f}")
PY
        ) || { echo "$name FAILED"; STATUS=1; return; }
        if [ -z "$best" ] || python3 -c "exit(0 if $t < $best else 1)"; then best="$t"; fi
    done
    printf "%-24s %ss\n" "$name" "$best"
}

echo "== micro (best of $N)"
for f in bench/*.soc; do
    run_target "$(basename "$f" .soc)" "$BIN" "$f"
done

echo "== macro (best of $N)"
run_target lisp        "$BIN" demos/lisp/main.soc
run_target checkers    "$BIN" demos/checkers/main.soc
run_target wfc         "$BIN" demos/wfc/main.soc
run_target sudoku      "$BIN" demos/sudoku/main.soc
run_target reversi     "$BIN" demos/reversi/main.soc
run_target spectra     "$BIN" demos/spectra/main.soc
run_target png         "$BIN" demos/png/main.soc
# spec_suite is valid ONLY single-tree: its sources move with each ref,
# so a cross-binary comparison would time different suites, not the
# binary. It has a row here because run.sh is single-tree by
# construction; it is deliberately absent from ab.py's targets.
run_target spec_suite  "$BIN" test tests/spec

if [ "$STATUS" -ne 0 ]; then
    echo "one or more targets FAILED" >&2
fi
exit "$STATUS"
