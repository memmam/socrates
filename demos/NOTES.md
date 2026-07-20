# Field-test notes: what ten demos taught the language

The demos in this directory were written against **v0.5** by ten authors
working independently, each with the same brief: build something real, use
the prebuilt interpreter, pin your output with `socrates test` directives, and
report every place the language fights you. Each demo was then verified by
a separate reviewer following only its README. This file is the triage of
their combined reports — every issue, and what happened to it.

The one-line summary: ten authors hit the same dozen walls, which is the
strongest possible signal. Those walls became **v0.6**.

## Bugs — fixed in v0.6

| Issue | Reported by | Fix |
|-------|-------------|-----|
| `math.seed` collapsed adjacent seeds: state was `seed \| 1`, so seeds 2k and 2k+1 produced **identical** streams (seed-42 and seed-43 dungeons compared equal), and nearby seeds barely diverged. | dungeon (its "different seeds ⇒ different dungeons" golden test failed) | Seeds now pass through a SplitMix64 scramble (`src/vm.rs`). Same seed still reproduces exactly; adjacent seeds are unrelated. Seeded streams are **not** stable across releases — dungeon and wfc re-pinned their goldens. |
| The `socrates test` directive scanner matched `//?` anywhere in a line — including inside string literals and in the prose of ordinary comments — injecting phantom expectations with a baffling "output mismatch". Three authors were bitten by comments *about* directives. | lisp, regex, sudoku | A directive now counts only when `//?` **begins the line's comment**, with enough string-awareness to skip `//` inside quotes (`src/testing.rs`); `tests/spec/lexical/directive_scanner.soc` pins it. |
| Struct-literal field shorthand `P { x, y }` was implemented (and `socrates fmt` even canonicalized *into* it) but absent from the normative spec — five authors independently flagged the mismatch and avoided the feature. | lisp, regex, dungeon, checkers, wfc | Documented in SPEC §2.3 and the grammar. (One report claimed shorthand *failed* to parse inside a function body; that repro was tested and works — misattributed.) |

## Ergonomics — added in v0.6

| Ask | Reported by | What was added |
|-----|-------------|----------------|
| Tuple destructuring in `for` heads (`enumerate`/`zip`/`entries` all yield pairs; every loop cost a `let (a, b) = pair;` line) | **8 of 10 demos** — the most-reported issue | `for` heads take any irrefutable pattern: `for (i, x) in xs.enumerate()`, nested tuples, `_`. |
| Bare `return`/`break`/`continue` as match-arm bodies (interpreters early-exit from arms constantly; the block form was pure ceremony and the parse error cascaded) | lisp, regex | Legal as sugar for the one-statement block. Assignment arms stay errors, now with a targeted hint. |
| `while true { .. }` not recognized as diverging (functions needed a dead `None // unreachable` after the loop — std/iter.soc itself carried the wart) | wfc + its verifier | A trailing escape-free `while true` diverges; `os.exit` also typechecks anywhere, like `panic`. |
| `_` as a discard binder in lambdas and loops | checkers, sudoku, wfc | `(0..81).map(\|_\| 0)` and `for _ in 0..3` both work. |
| `trim_start` / `trim_end` (four demos hand-rolled an O(n²) rtrim from `slice` to keep goldens clean) | lisp, spreadsheet, dungeon, checkers, wfc | Added, Unicode-whitespace, alongside `trim`. |
| Fixed-precision float formatting (three demos hand-rolled multiply-round-divide, which misrounds 1.005 and can't pad) | spreadsheet, csvql, plot | `Float.to_fixed(n)` — exactly n places, no `-0.00`. |
| Character ↔ code-point conversion (alphabet lookup tables were the only route to "column letter to index") | spreadsheet, regex | `s.code_at(i) -> Option[Int]` and free `char(code) -> String`. |
| Substring search from an offset (hand-written scanners exploded strings into char lists to get a cursor) | mdsite | `s.index_of_from(pat, from)` — char indices, like every string index. |
| Integer randoms (`lo + (random() * span).to_int()` is easy to get subtly wrong) | dungeon, wfc | `math.rand_int(lo, hi)`, inclusive, panics on an empty range. |
| `math.log10`, float remainder | plot | `math.log10`, `math.fmod`. `%` stays Int-only by design. |
| The fixed 4,096-frame call-depth cap (deep spreadsheet dependency chains and long regex inputs hit it legitimately) | spreadsheet, regex | `SOCRATES_MAX_DEPTH` env var (floor 64). The default stays; `try()` still catches overflow. |

## Diagnostics and tooling — sharpened in v0.6

- `let m: Map[..] = {};` now says **"`{}` is an empty block, not an empty
  map"** with the `{:}` note, instead of a generic Unit-vs-Map mismatch
  (csvql).
- `Some(v) -> i = v` now says assignment can't be an arm body and shows the
  block form (mdsite).
- `x << 1` now gets "Socrates has no bitwise shift operators" from the lexer
  instead of a bare "expected an expression" (sudoku).
- `socrates test` golden comparison ignores trailing whitespace on both sides —
  trailing spaces in a directive are invisible in an editor and could never
  be pinned reliably (checkers).
- `socrates test --help` (or any unknown flag) prints usage and exits 64
  instead of silently testing the whole working directory (dungeon's
  verifier).
- Spec gaps closed in `docs/SPEC.md`: `sort`/`sort_by` documented **stable**
  (csvql — deterministic `order by` ties silently depended on it),
  `math.log` documented as natural log (plot), match-arm divergence
  documented (spreadsheet), field shorthand documented (above).

## Heard, and declined — with reasons

- **Bitwise operators** (sudoku wanted 9-bit candidate masks). One demo,
  one use case, and Bool tables carried it fine. The `<<` diagnostic keeps
  the door marked. Waiting for more pull. (The pull arrived: `&
  | ^ << >>` shipped in v0.7 — see CHANGELOG.md's v0.7.0 entry — and by
  the v0.7 round nine of the then-seventeen demos used them; this entry
  is kept as the historical record of why the v0.6 decision was "not
  yet," not as a currently-accurate status.)
- **Multi-line / raw string literals** (mdsite generated HTML with twenty
  `push` calls, in its v0.6 shape — since rewritten around
  `strings.Builder`, so that exact figure is no longer reproducible
  against current code, but the friction it illustrates is unchanged:
  every literal `{` in CSS needs `\{`). Real friction, but a
  design decision with interpolation interactions — deferred whole, not
  half-designed.
- **A line-width-aware formatter.** The single most-reported *tooling*
  issue: `socrates fmt` collapses deliberate multi-line literals (a 57-entry
  test table became one 1,427-char line). This is a genuine
  limitation and staying on the list — but it's a formatter rewrite, not a
  papercut fix, and the formatter remains *correct* (idempotent,
  behavior-preserving). None of the demos are fmt-clean; that's honest.
  (Two sibling complaints in this same original report — hoisting comments
  out of list literals, and half-expanding if/else-if chains — are fixed;
  see "Bugs — fixed in-round," below. This entry now covers only the
  multi-line-literal-collapsing complaint, which is still open.)
- **A `Set` type / `min_by`/`max_by` / `List.fill` / `sum`/`min`/`max` /
  a string builder / a deque.** Each was worked around in one line or two
  (`Map[K, Bool]`, a fold, `(0..n).map(|_| v)` — nicer now with `_`, a
  list-plus-cursor queue). (The pull arrived: `std.set`, `lists.fill`/
  `sum`/`min`/`max`/`min_by`/`max_by`, `strings.Builder`, and `std.deque`
  all shipped in v0.7 — see CHANGELOG.md's v0.7.0 entry and `docs/SPEC.md`
  §7.1; this entry is kept as the historical record of why the v0.6
  decision was "not yet," not as a currently-accurate status.)
- **`socrates test` counts directive-less files as passing** (lisp's
  verifier). Kept: "a file with no directives must run silently" is
  documented semantics and useful for smoke-running libraries; a mistyped
  directive fails loudly the moment output exists.
- **`os.exit(0)` after a failed load in a demo** (spreadsheet's verifier
  nit): demo-level choice, left to the demo.

## The review round

Before shipping, four adversarial reviewers attacked the v0.6 interpreter
diff itself. Confirmed findings, all fixed in the same release:

- The first directive-scanner fix mis-tracked strings nested inside
  interpolation holes (a boolean toggle), which could make a **real
  directive silently vanish** — a false *pass* — and wasn't
  block-comment-aware either. The scanner now models strings, holes (to
  arbitrary depth), and nested `/* */` comments across lines, and the edge
  cases are pinned in `tests/spec/lexical/directive_scanner_edge.soc`.
- Typing `os.exit` like `panic` regressed the REPL: a bare `os.exit(0)`
  produced "cannot infer the type of `__repl_1`" instead of exiting. The
  panic-result Unit-defaulting rule now covers it.
- `math.rand_int` used a widening-multiply reduction without rejection —
  measurably non-uniform for spans above 2^32. Now Lemire's method *with*
  rejection: exactly uniform.
- A valueless bare `return` arm with the comma omitted swallowed the next
  arm's pattern as its "value" and produced misleading cascades (never a
  silent misparse); it now gets a targeted diagnostic. Compound
  assignments (`+=`) in arm bodies get the same hint plain `=` does.
- `socrates fmt` erased the new arm sugar back to block form; arms remember
  they were written bare and round-trip. `socrates test --` ends flag
  parsing; malformed `SOCRATES_MAX_DEPTH` warns instead of silently using
  the default (and the SPEC documents the native-callback caveat of huge
  caps).

## Performance notes (informational, no action)

- lisp: double interpretation runs ~40–50k evals/sec in the release build;
  the 100k-iteration tail-recursive Lisp loop dominates its 2.3s runtime —
  and runs in constant stack because Socrates's TCO reaches through `eval`.
- checkers: ~38k search nodes/sec (507k nodes for the 106-ply game, ~13.5s)
  using the apply/undo pattern over one shared board.
- wfc: full-rescan WFC over 4,800 cells took 19.5s (~3–6M interpreted
  ops/sec); map lookups with interpolated string keys measured at ~0.36s
  per million — composite string keys are a fine idiom.
- Everything above survives `SOCRATES_GC_STRESS=1` byte-identically.

---

## The v0.7 round

The process re-ran against **v0.7** (the infrastructure release): six new
demos exercising Bytes/FFT/workers/bitwise/std-collections, plus a
modernization pass over all eleven existing demos — seventeen independent
authors, seventeen adversarial verifiers, every demo green under GC stress
before integration. Distilled best practices: [`STYLE.md`](STYLE.md).

(Correction, 2026-07-19: "six new... eleven existing" undercounts by one
on both sides. `git log --follow --diff-filter=A` against each
`demos/*/` directory shows the v0.6 field test shipped exactly ten
demos (not eleven), and the v0.7 round's actual new demos were
`synthwave`, `png`, `bloom`, `spectra`, `swarm`, `reversi`, **and
`parmandel`** — seven, not six; `parmandel` was first-committed at the
same v0.7.0 tag as the other six but never named in this round's own
prose anywhere it appears (this file, CHANGELOG.md, STYLE.md,
`demos/README.md`). Total demos touched in the round is unchanged at
seventeen either way (7 new + 10 existing = 17 = 6 + 11), which is why
this went unnoticed for so long — every "seventeen" count downstream
of this sentence stayed correct even though the six/eleven breakdown
feeding it was wrong. The original sentence above is left as shipped;
this note is the correction, not a replacement for it — CHANGELOG.md's
matching v0.7.0 entry carries the same original-preserved-plus-noted-
correction treatment, pointing back here for the detail rather than
repeating it. STYLE.md's opening line got the opposite treatment,
correctly: it's a living reference to current best practice ("as
designed to now"), not a dated historical record, so its "seven new, ten
modernized" was corrected in place rather than preserved-plus-noted.)

### Bugs — fixed in-round

| Issue | Reported by | Fix |
|-------|-------------|-----|
| Method call on a module-qualified `pub let` member misresolved as an enum path (`m.answer.to_float()` → E0413 "no enum `answer`"; parens didn't help) | synthwave, checkers | The `alias.Enum.Variant` special case now falls through to ordinary field/method dispatch when the middle segment is a value member; E0413 fires only when it is neither. Regression: `tests/spec/module_system/module_member_methods.soc` |
| `worker.spawn` resolved relative files against the wrong directory whenever the entry script had any import (base came from `sources[0]` = first *loaded* module) | swarm | `Vm.entry_dir` is now set explicitly by every runner; spawn resolves against the true entry script's directory. Regression: `tests/spec/workers/spawn_with_import.soc` |
| `socrates fmt` silently formatted only its first file argument (exit 0, no diagnostic) | synthwave, reversi, spreadsheet, csvql | `socrates fmt [-w] [--width N] <file>...` now formats every argument |
| The formatter evicted comments living inside bracketed literals, dumping them orphaned after the statement | reversi, regex, csvql | Interior comments now pin the broken element-per-line layout with each comment kept in place — which is also the official escape hatch for meaning-bearing 2-D layout (wfc's training samples) |
| Value-position `if/else-if` chains broke despite fitting in 100 columns, and asymmetrically (first branch blockified, rest inline) | synthwave, reversi, csvql | Chains that fit stay on one line; chains that don't break all branches consistently |

### The feature queue (deduplicated, by demand)

Reported as missing by independent authors; counts are distinct demos.
These are the v0.8 candidates, roughly in order of observed pain:

- **Bit intrinsics** (×5: bloom, dungeon, sudoku, wfc, reversi):
  `count_ones`/`trailing_zeros`/`leading_zeros` on Int. Every bit-heavy
  demo hand-rolls SWAR popcount and binary-search ctz, and the classic
  shortcuts (`x & -x`, multiply folds) panic under checked overflow.
- **Bytes readers + BE + bulk append** (×5: synthwave, png ×2, bloom,
  regex): the LE pushers have no matching readers; big-endian formats
  (PNG) hand-roll everything; no `push_bytes`/`push_str` bulk append
  (concat is O(total) per join), no 64-bit accessors.
- **Logical right shift** (×3: reversi, sudoku, checkers-docs): `>>` is
  arithmetic; unsigned-bitfield code re-masks after every shift, and the
  obvious mask `(1 << 64 - k) - 1` panics at k = 1. A `>>>` operator or
  `ushr` intrinsic; `reversi/bits.soc` is the interim reference.
- **Bitwise compound assignment** (×3: reversi, sudoku, wfc): `|=` `&=`
  `^=` `<<=` `>>=` to match the arithmetic set.
- **`while let` / `if let`** (×3: dungeon, mdsite, parmandel): the
  deque-drain and worker-recv loops force an `is_empty`+`unwrap` or
  6-line `while true { match ... }` dance.
- **Hex** (×3: png, bloom + literals in reversi): no `to_hex`, no hex
  `parse_int`, and Int literals can't express bit patterns ≥ 2⁶³
  (`0x8080808080808080` is unwritable; reversi builds masks by shifting).
- **Builder ergonomics** (×3: spreadsheet, mdsite, plot): `is_empty()`,
  a separator-aware `build(sep)`/join mode.
- **fft magnitude helper** (×2: spectra, plot): every rfft consumer
  writes the same `re.zip(im).map(|p| ...)` power/magnitude line.
- Singles worth noting: worker `try_recv`/select (swarm — the dynamic
  scheduler workaround is documented in its README), `lists` key-based
  `max_by_key` (spectra), `socrates test --bless` re-pinning mode (bloom),
  module-level constants / lazy statics (lisp), a counting-map helper
  (checkers), `Range.all/any` (sudoku), 32-bit wrapping multiply for
  hash finalizers (bloom), ergonomic `std.json` construction (swarm).

### Warts acknowledged, working as intended (documented instead)

- Named `fn` declarations always take block bodies under the formatter;
  the "fits stays flat" rule applies to expressions, not items (SPEC).
- A call whose single argument is an over-width string literal stays on
  one over-width line rather than breaking pointlessly (fmt keeps
  "never split a token" and doesn't wrap what wrapping can't fix).
- `import std.lists` binds the name `lists` (the final segment), per the
  module system's normal rule; SPEC prose spells it `std.lists` when
  naming the module path. A targeted diagnostic for `std.X` is queued.
- Seeded `math.seed` streams remain stable within a release only —
  corpora that must outlive releases belong to hand-rolled PRNGs
  (STYLE.md § 2).

---

## The v0.8 round

The feature queue above was worked through directly rather than via a
fresh demo round. Four items in it were already stale by the time it was
written: `count_ones`/`leading_zeros`/`trailing_zeros`, `ushr`,
`rotate_left`/`rotate_right`, `to_hex`, and the Bytes BE pushers/readers +
`push_bytes`/`push_str` bulk append all landed within v0.7 itself (its late
efficiency pass pulled them forward — CHANGELOG.md's v0.7.0 entry has
them, this file's queue didn't get updated to match). Confirmed present before
starting v0.8, not re-done.

Genuinely resolved in v0.8:

| Queue item | Resolution |
|------------|------------|
| Bitwise compound assignment | `\|= &= ^= <<= >>=`, matching the arithmetic set — Int-only, never dispatches, exactly like the plain bitwise operators |
| `while let` / `if let` | Parser-level sugar, desugared fully to `match` at parse time (`if let` → a two-arm match; `while let` → `while true { match .. { _ -> break } }`, the exact hand-written idiom above) — so the checker and compiler need no special cases, and an irreducible user pattern making the synthetic fallback arm unreachable is silently fine, not a warning |
| Hex: bit patterns ≥ 2⁶³, `parse_hex` | Hex/binary literals now parse as the raw 64-bit pattern (`0x8080808080808080` and `0x8000000000000000` — `Int`'s minimum — are both writable); `String.parse_hex()` is `to_hex()`'s inverse |
| Bytes 64-bit accessors | `push_u64le`/`push_u64be`/`read_u64le`/`read_u64be` — no range check needed at 64 bits, since `Int` already *is* the two's-complement value |
| Builder ergonomics | `is_empty()`, `push_joined(sep, s)` (pushes `sep` first unless this is the builder's first piece — the manual "gate on `len() > 0`" idiom, wrapped) |
| fft magnitude helper | `fft.magnitude(re, im) -> List[Float]` |
| worker `try_recv` | Non-blocking `recv`, `Option[Option[String]]` (not-ready / hung-up / message) — covers the polling need directly; no separate `select` |
| `lists` key-based `max_by_key`/`min_by_key` | Int-valued key extractor, alongside the existing comparator-based `max_by`/`min_by` |
| `socrates test --bless` | Rewrites mismatched `//? expect:` lines in place when the actual/expected line count already agrees; a count change (a print statement added or removed) still fails normally — deciding which new line pairs with which directive needs a human |
| module-level lazy statics | `std.lazy`: `Lazy[T]`, `of(thunk)`, `.get()` (computes once, caches), `.is_forced()`. (Eager module-level `let` already built once at import, per STYLE.md § 6 — this adds the deferred half.) |
| `Range.all`/`any` | Short-circuiting, matching `List`'s; previously reachable only via `.to_list().any(..)` |
| 32-bit wrapping multiply | General `wrapping_add`/`wrapping_sub`/`wrapping_mul` (64-bit) — a 32-bit wrap is `a.wrapping_mul(b) & 0xFFFFFFFF`; one primitive, not a second 32-bit-specific intrinsic |
| ergonomic `std.json` construction | `json.obj`/`arr`/`jstr`/`num`/`int`/`bool`/`null` — the same tree as the raw `Json.J*` constructors, named for what they build (`jstr`, not `str`: this module's own code calls the builtin `str()`, and a same-named local function would shadow it for every unqualified call in the file — hit and fixed while writing this) |

### Heard, and declined — with reasons

- **A counting-map helper** (checkers). One demo, and `m.insert(k,
  m.get(k).unwrap_or(0) + 1)` is a single line with no repeated pattern
  across the codebase to justify a named primitive. `std` grows reluctantly
  (v0.6/v0.7's own rule) — revisit if more than one demo asks.

### The feature-queue adoption gap, closed (v0.8 minification pass, W1c)

The v0.8 queue's features shipped, but several of the demos that asked
for them never switched over — the hand-rolled versions stayed behind.
The minification pass's demo wave closed the gap, byte-identical goldens
throughout: `bloom`'s 16-bit-halves `mul32` became
`a.wrapping_mul(b) & 0xFFFFFFFF` (the exact case the intrinsic was added
for), `checkers`' masked-shift `lshr` became `x.ushr(k)` (the reversi
precedent, finally applied), `spreadsheet` and `mdsite` adopted
`push_joined` (`mdsite`'s document-level `block()` join, missed in that
wave, followed in the consistency pass below — the claim was fully true
at the time; that `block()` join has since moved with the rest of
`mdsite/markdown.soc` into `std.markdown`, v0.8, so the code demonstrating
it now lives in `std/markdown.soc` rather than the demo itself), `swarm`
adopted the `std.json` constructors that were
added *for it* (`json.int`/`jstr`/`obj`), `regex` reads its bitmap words
in one `read_u64le`, and the four Int-key comparator sites (`reversi`,
`swarm`, `bloom`, `dungeon`) moved to `max_by_key`. The lesson for
future feature queues: adoption is part of shipping — a queue item isn't
done until the demo that motivated it uses it.

### The deliberate-divergence ledger (STYLE.md R4, consistency pass)

STYLE.md § 9 named its rules (R1-R4) so a site that deliberately
diverges can cite the rule it breaks; this ledger mirrors every
in-place divergence comment. The consistency pass that introduced R4
also swept the demos to conformance (compound bitwise assignment,
`if let`/`while let`, `push_joined`, aggregate/extreme/std idioms —
byte-identical goldens except two deliberate re-pins of prose inside
printed lines: `bloom`'s popcount line now names the `count_ones`
intrinsic, `synthwave`'s decoded-header line now names the native LE
readers). Standing divergences:

- `bloom/bloom.soc` `bit_string` — diverges from § 6's `push_joined`
  rule: the body is an alloc-free `push_char` loop and the manual
  separator gate keeps it allocation-free; `push_joined` would need a
  String piece per byte.
- `wfc/wfc.soc` `lowest_entropy` — R1 hot-path exemption: the
  hottest loop in the demo; `enumerate()`/`min_by` allocate per
  element, the hand scan with two locals allocates nothing. The same
  file's `support()` keeps `r.mask(s, d)` inlined (with its `|=`
  accumulator) for the same hot-loop reason, per its in-place comment.
- `sudoku/solver.soc` `best_cell` — R1 early-exit exemption: the
  scan breaks as soon as a 2-candidate cell appears, which no
  aggregate spelling can express.
- `checkers/engine.soc` `best_move` — R1 exemption: each candidate's
  score is a full apply/negamax/undo on the shared board interleaved
  with the alpha update; an aggregate cannot express the side effects
  or the pruning.
- `swarm/crunch_worker.soc` `collatz_champion` — R1 allocation
  exemption: `max_by_key` would materialize a `(n, steps)` pair list
  per job inside the worker's hot loop.
