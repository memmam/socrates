#!/usr/bin/env bash
# Fable benchmark harness: micro benches (bench/*.fable) + macro benches
# (heavy demo mains). Each target runs N times; the MINIMUM wall time is
# reported (least-noise estimator). Usage: bench/run.sh [N]
# The binary must already be built: cargo build --release
set -u
N="${1:-3}"
BIN=./target/release/fable
command -v python3 >/dev/null || { echo "needs python3"; exit 1; }

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
        ) || { echo "$name FAILED"; return; }
        if [ -z "$best" ] || python3 -c "exit(0 if $t < $best else 1)"; then best="$t"; fi
    done
    printf "%-24s %ss\n" "$name" "$best"
}

echo "== micro (best of $N)"
for f in bench/*.fable; do
    run_target "$(basename "$f" .fable)" "$BIN" "$f"
done

echo "== macro (best of $N)"
run_target lisp        "$BIN" demos/lisp/main.fable
run_target checkers    "$BIN" demos/checkers/main.fable
run_target wfc         "$BIN" demos/wfc/main.fable
run_target sudoku      "$BIN" demos/sudoku/main.fable
run_target reversi     "$BIN" demos/reversi/main.fable
run_target spectra     "$BIN" demos/spectra/main.fable
run_target png         "$BIN" demos/png/main.fable
run_target spec_suite  "$BIN" test tests/spec
