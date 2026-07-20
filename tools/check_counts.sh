#!/usr/bin/env bash
# Prose counts vs. the live suite.
#
# The count-bearing sentences — the six spec-count places CLAUDE.md's
# workflow conventions enumerate (README ×2, CLAUDE.md ×1, PROJECT.md ×1,
# RELEASE_NOTES.md ×1, book/11-toolchain.md ×1), the book
# executed-of-total claim, the demo-suite count, and the spelled-out
# demo-program count — are each extracted by an exact anchor below and
# compared against the live totals. A release draft once shipped saying
# 311 while the suite stood at 313 (and book/11-toolchain.md drifted to
# the same wrong number, silently, since it wasn't yet one of the
# enforced places); this script turns that class of drift into a red X
# (CI runs it in the Test job; the gauntlet runs it locally).
#
# The anchors are deliberately exact: rewording a counted sentence makes
# its extraction come back empty and fail loudly — re-anchor it here in
# the same PR that rewords the prose.
set -u
cd "$(dirname "$0")/.."
bin="${1:-./target/release/socrates}"
fail=0

check() { # check <label> <live> <claimed-from-prose>
  if [ -z "$3" ]; then
    echo "FAIL $1: anchor matched nothing (prose reworded? re-anchor here)"
    fail=1
  elif [ "$2" != "$3" ]; then
    echo "FAIL $1: live=$2 prose=$3"
    fail=1
  else
    echo "ok   $1: $2"
  fi
}

live_spec=$("$bin" test tests/spec 2>&1 >/dev/null \
  | sed -n 's/^ok: \([0-9]\{1,\}\) tests passed$/\1/p')
shopt -s extglob
live_demo=$("$bin" test demos/!(glcube)/ demos/glcube/cube.soc demos/glcube/spec.soc 2>&1 >/dev/null \
  | sed -n 's/^ok: \([0-9]\{1,\}\) tests passed$/\1/p')
live_book_total=$(grep -rhE '^[[:space:]]*```soc' book/ | wc -l | tr -d ' ')
live_book_skip=$(grep -rhE '^[[:space:]]*```soc[[:space:]]+skip' book/ | wc -l | tr -d ' ')
live_book_exec=$((live_book_total - live_book_skip))
live_demo_dirs=$(ls -d demos/*/ | wc -l | tr -d ' ')

[ -n "$live_spec" ] || { echo "FAIL: no live spec count (suite red or binary missing?)"; exit 1; }
[ -n "$live_demo" ] || { echo "FAIL: no live demo count (suite red or binary missing?)"; exit 1; }

# --- spec-suite count: the five places -------------------------------
check "README 'N golden spec tests'" "$live_spec" \
  "$(sed -n 's/^\([0-9]\{1,\}\) golden spec tests.*/\1/p' README.md)"
check "README 'own N-test suite'" "$live_spec" \
  "$(sed -n 's/.*own \([0-9]\{1,\}\)-test suite.*/\1/p' README.md)"
check "PROJECT 'tests/spec/, N tests'" "$live_spec" \
  "$(sed -n 's/.*`tests\/spec\/`, \([0-9]\{1,\}\) tests.*/\1/p' PROJECT.md)"
check "CLAUDE gauntlet '# N'" "$live_spec" \
  "$(sed -n 's/.*socrates test tests\/spec *# \([0-9]\{1,\}\)$/\1/p' CLAUDE.md)"
check "RELEASE_NOTES 'pinned: N golden spec tests'" "$live_spec" \
  "$(sed -n 's/.*pinned: \([0-9]\{1,\}\) golden spec tests.*/\1/p' .github/RELEASE_NOTES.md)"
check "book/11-toolchain.md spec suite (N tests)" "$live_spec" \
  "$(sed -n "s/.*own spec suite (\([0-9]\{1,\}\) tests).*/\1/p" book/11-toolchain.md)"

# --- demo-suite count ------------------------------------------------
check "CLAUDE gauntlet demos '# N'" "$live_demo" \
  "$(sed -n 's/.*spec\.soc *# \([0-9]\{1,\}\)$/\1/p' CLAUDE.md)"
check "RELEASE_NOTES 'N demo golden tests'" "$live_demo" \
  "$(sed -n 's/.*and \([0-9]\{1,\}\) demo golden tests.*/\1/p' .github/RELEASE_NOTES.md)"

# --- book snippet counts (fence census, same classes as the harness) --
check "README 'a book N of'" "$live_book_exec" \
  "$(sed -n 's/.*a book \([0-9]\{1,\}\) of$/\1/p' README.md)"
check "README 'executable book: N of the M' (executed)" "$live_book_exec" \
  "$(sed -n 's/.*\*\*An executable book\.\*\* \([0-9]\{1,\}\) of the [0-9]\{1,\}.*/\1/p' README.md)"
check "README 'executable book: N of the M' (total)" "$live_book_total" \
  "$(sed -n 's/.*\*\*An executable book\.\*\* [0-9]\{1,\} of the \([0-9]\{1,\}\).*/\1/p' README.md)"
check "PROJECT 'N of M execute' (executed)" "$live_book_exec" \
  "$(sed -n 's/^ *\([0-9]\{1,\}\) of [0-9]\{1,\} execute.*/\1/p' PROJECT.md)"
check "PROJECT 'N of M execute' (total)" "$live_book_total" \
  "$(sed -n 's/^ *[0-9]\{1,\} of \([0-9]\{1,\}\) execute.*/\1/p' PROJECT.md)"
check "RELEASE_NOTES 'N executed book'" "$live_book_exec" \
  "$(sed -n 's/.*, \([0-9]\{1,\}\) executed book.*/\1/p' .github/RELEASE_NOTES.md)"

# --- demo-program count (stated in words) ----------------------------
case "$live_demo_dirs" in
  15) word=fifteen ;;   16) word=sixteen ;;   17) word=seventeen ;;
  18) word=eighteen ;;  19) word=nineteen ;;  20) word=twenty ;;
  21) word=twenty-one ;; 22) word=twenty-two ;; 23) word=twenty-three ;;
  24) word=twenty-four ;; 25) word=twenty-five ;;
  *) echo "FAIL demo-program word table: $live_demo_dirs demos — extend the case table"; fail=1; word='' ;;
esac
if [ -n "$word" ]; then
  # Occurrence counts are anchors too: adding or removing a sentence
  # that states the demo-program count updates the two numbers here.
  check "README lines saying '$word'" 4 "$(grep -ci "$word" README.md)"
  check "RELEASE_NOTES lines saying '$word'" 1 "$(grep -ci "$word" .github/RELEASE_NOTES.md)"
  check "demos/README 'N programs'" "$word" \
    "$(sed -n 's/^\([A-Za-z-]*\) programs,.*/\1/p' demos/README.md | tr '[:upper:]' '[:lower:]')"
fi

exit "$fail"
