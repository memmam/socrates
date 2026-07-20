# Socrates demo style — best practices as designed to now

Distilled from the v0.7 demo round: seventeen programs (seven new, ten
modernized), each written by an independent author under field-test
orders, each adversarially verified. Where a rule cites a demo, that
demo is the reference implementation of the rule. The papercut ledger
behind these rules lives in `NOTES.md`.

This file is **normative** for `demos/` (and, per R3, for ports'
internals): the demos conform to it, not the other way around. Rules a
site may deliberately diverge from carry names (R1-R4, § 9) so the
divergence comment can cite its rule; `NOTES.md` keeps the ledger of
every such divergence.

## 1. Golden discipline

- **Pin everything, and make pins self-checking.** A golden line should
  carry its own verdict where possible: print `-> true` Bools computed
  from load-bearing comparisons (regeneration equality, checksum
  verification, detected-bin match) so a future wholesale re-pin of
  drifted output still fails on the semantic checks (`synthwave`).
- **Pin published constants, not just round-trips.** A CRC-32 that
  checks itself proves consistency, not correctness; `crc32("IEND") ==
  ae426082` and `adler32("Wikipedia") == 11e60398` tie the code to
  numbers it cannot invent (`png`). Same idea: Othello movegen proven by
  published perft values 4/12/56/244/1396/8200 before anything else was
  pinned (`reversi`).
- **Generate long pin blocks mechanically.** Run the program, pipe
  through `sed 's/^/\/\/? expect: /'`, append, re-run `socrates test`
  (golden comparison already ignores trailing whitespace on both
  sides, so the sed pipeline needn't strip it). Never hand-transcribe a transcript
  (`spectra`, `reversi`, `swarm`). Verify expected values against an
  independent reference *before* pinning. For an existing pin that's
  merely drifted — the line count of actual vs. expected already
  agrees, only the values changed — `socrates test --bless` rewrites
  the mismatched `//? expect:` lines in place; reach for the sed
  pipeline only when generating a pin block from scratch or when the
  line count itself needs to change.
- **Deliberate panics get their own tiny file** — a panic ends the run,
  so nothing after it prints. Pin extra contract checks in the main file
  through `try(|| ...)` with the message as a normal expect line
  (`bloom` guardrails, `spectra`).
- **Print-free modules are free tests.** A library `.soc` with no
  top-level statements passes the harness as a silent no-directive file
  — keeping demo modules print-free is both hygiene and coverage.

## 2. Determinism

- **Never pin raw libm output** (`sin`/`cos`/`exp`/`log`/`pow`/`tanh`,
  `sqrt` of irrationals). Compare in squared magnitude or integers;
  convert with `.to_fixed(k)` only at the print boundary (`spectra`).
  Plain `+ - * /` f64 arithmetic is exactly deterministic and safe to
  pin raw.
- **The exact-bin FFT recipe:** choose n and the sample rate so
  `freq * n / sr` is an integer — one-second windows make bin k exactly
  k Hz. No leakage, so bin indices and to_fixed magnitudes pin exactly;
  demand a dominance margin (>5×) before trusting argmax across libm
  implementations (`spectra`, `synthwave`).
- **Audit half-boundaries** before pinning anything through `round` or
  `to_fixed`: pick display constants so nothing lands on x.5, or print
  with one more digit so the boundary moves away from the value
  (`spectra`).
- **Corpora that must outlive releases come from your own PRNG** (LCG /
  xorshift32 in plain Socrates integer ops), not `math.seed` — seeded
  streams are stable only within a release. Pick a PRNG family different
  from any hash under test (`bloom`).
- **Committed binary artifacts want an all-integer signal path.** Phase
  accumulators, geometric decay by repeated multiplication, LFSR noise,
  rational sine approximations — exactly deterministic by construction,
  so `fs.read_bytes(committed) == built` pins byte-for-byte on every
  machine (`synthwave`, `png`).

## 3. Bytes and binary formats

- **Write with the LE pushers, read with the LE readers** — the two
  directions verify each other (`synthwave` encodes with `push_u32le`
  and decodes its own header back out with `read_u32le`/`read_u16le`;
  `read_i16le` hands back the sample already sign-folded). The
  `get()`-shift-or reassembly and the hand sign-fold these replaced
  live in git history.
- **The regeneration pattern:** read the committed artifact *before*
  rewriting it, pin `committed == fresh_build` (structural `Bytes ==`
  makes this a one-line golden), then write — the artifact self-heals
  during development and proves cross-machine determinism in CI
  (`png`, `bloom`, `plot`, `mdsite`).
- **Treat Int as u32 by invariant, not by masking everywhere:** keep
  values in 0..2³²−1 and arithmetic `>>` never sign-extends because bit
  63 is never set; mask only to isolate sub-fields (`png`).
- **Add a corruption drill.** Flip one bit and pin that exactly the
  right checksums fail — it proves the verifier isn't vacuous (`png`;
  `sudoku`'s corrupt-board check is the same idea one level up).

## 4. Bitwise house rules

- Precedence is Rust's: most bitboard/checksum expressions read
  paren-free (`x >> n & mask`, `acc | run & empty`, `x & m != 0`,
  `table[(c ^ b) & 255] ^ c >> 8` — `std.crc`, moved from `png`) — but
  `& | ^` bind *looser* than `+ -`, so `(v & 0x7f) + top` needs its
  parens. `socrates fmt` strips merely-clarifying parens, so precedence
  knowledge is not optional; comment intent instead.
- **`>>` is arithmetic.** Any value that can carry bit 63 goes through
  the `Int.ushr(n)` intrinsic (logical shift, `>>`'s panic contract).
  Never hand-build the mask — the textbook `(1 << 64 - n) - 1` panics
  at n = 1 (`reversi/bits.soc` documents every trap; its hand-rolled
  bodies live in git history).
- **Overflow panics disable classic bit tricks.** `x & -x` and
  `x & (x - 1)` both panic on the bit-63-only value; iterate set bits
  with `trailing_zeros` + `^` (which also yields ascending order — a
  free deterministic tie-break). Counting is `count_ones()` — never
  hand-roll a popcount. When a bit algorithm assumes wrapping
  arithmetic, assume it is broken in Socrates until proven otherwise
  (`reversi`).
- **32-bit hashing rules:** mask `& 0xFFFFFFFF` after every add/multiply
  that can exceed 32 bits; split full 32×32 multiplies into 16-bit
  halves; write the invariants in a header comment because the types
  can't express them (`bloom`).
- **9-bit fields** (the house speciality): full mask is `(1 << 9) - 1 =
  511`, set algebra is `(row | col | box) ^ 511`, singles are
  `popcount(m) == 1`, and guesses peel lowest-bit-first for determinism
  (`sudoku`).

## 5. Workers

- **Determinism by protocol, never by luck.** Two pinnable regimes:
  static assignment + drain one worker at a time (per-handle FIFO makes
  even per-worker stats pinnable), or any dynamic schedule + aggregate
  by job id into fixed order, pinning nothing that names a worker — the
  second survives scheduler rewrites without re-pinning (`swarm`).
- **Channels buffer: deal every job up front, then drain.** Full pool
  parallelism, zero synchronization code, and the parent's send loop
  never blocks (`swarm`, `parmandel`).
- **The worker main loop** is `while let Some(msg) = worker.recv() {
  ... }` (v0.8 sugar; it desugars at parse time to exactly the old
  `let msg = match worker.recv() { Some(m) -> m, None -> break };`
  form, `swarm/crunch_worker.soc`). Support both an explicit quit
  message and `None`-means-quit so workers never hang regardless of how
  the parent leaves.
- **Workers never `println`.** All output flows through the channel and
  the parent prints in protocol order. Helper files guard everything
  behind `if worker.is_worker() { ... }` so standalone harness runs are
  silent (`swarm/crunch_worker.soc`, `parmandel/row_worker.soc`).
- **Panic isolation is a feature to demo, not just tolerate:** `join()`
  returns the panic message verbatim, `send` returns false on the dead
  handle, and jobs-as-messages makes "respawn and resend" a three-line
  recovery (`swarm`).

## 6. std collections idioms

- **`set.insert` returning Bool is the canonical first-sighting test:**
  `if !busy.insert(key) { /* cycle */ }` merges membership check and
  mark (`spreadsheet`'s `#CYCLE!`, `checkers`' threefold repetition,
  `lisp`'s duplicate params). Insertion-ordered `to_list()` gives
  deterministic "distinct, first-seen order" output with no sorting.
- **`min_by`/`max_by` keep the first winner on ties** — feed candidates
  in ascending order and "argmax with lowest-index tie-break" needs zero
  explicit tie-break code (`reversi`, `swarm`, `spectra`).
- **Thread ONE `strings.Builder` through a recursive printer** (allocate
  at the top, pass `&`-style through a `write_x(value, b)` recursion)
  instead of returning Strings for parents to join — O(output) instead
  of O(output × depth) (`lisp`). For line-oriented output, emit the
  separator *before* each line gated on `len() > 0` (`spreadsheet`).
- **Join with `push_joined`, not a length gate** (v0.8): the
  separator-before-each-piece idiom is `b.push_joined(sep, s)` — it
  pushes `sep` first unless this is the builder's first piece, replacing
  the manual `if b.len() > 0 { b.push(sep); }` at every call site
  (`spreadsheet`, `mdsite`).
- **Don't force everything through the Builder:** short lines read
  better as `List[String].join(" | ")`; accumulate only the document in
  the Builder (`spreadsheet`).
- **Constant tables live in a context struct** that already threads
  through the computation — structs are references, so children copy a
  pointer. Never construct a Set/Map constant inside a hot function
  (`lisp`; module-level `let table = make_table();` builds once at
  import for module-private tables, `std.crc`, moved from `png`).

## 7. Formatter-aware authoring

- Run `socrates fmt -w` and re-run tests as the *last* step of every
  change; format before hand-polishing comments (the first pass on old
  files is a big canonicalization diff — polish after).
- **Comments belong above statements.** Interior comments in bracketed
  literals pin the broken (element-per-line) layout — that is the
  official escape hatch for meaning-bearing 2-D layout like `wfc`'s
  training samples. For annotated elements, either use interior
  comments deliberately or put an ordered legend above the literal.
- **Split big data tables into named sub-lists** that each fit the
  width and `.concat()` them — it survives the formatter and reads
  better anyway (`synthwave`'s bar-per-list melody tables).
- Whole-line `//? expect:` blocks are preserved verbatim; inline
  trailing directives may lose their column alignment.

## 8. Performance under GC stress

- CI runs everything under `SOCRATES_GC_STRESS=1`. Pure-integer,
  low-allocation demos are essentially free (`png`: 26 ms); transient
  allocation is what hurts (`checkers`, the heaviest demo in the suite:
  ~50 s). Budget pinned heavyweight loops accordingly and measure
  per-file (`lisp`'s 100k-iteration pin dominates its ~29 s stress run).
- Set-probe dispatch beat chained string compares under stress
  (`lisp`, −15%): fewer transient Options per call. Amortized lookups
  are a safe default even in hot paths.

## 9. Aggregates and std idioms (the named rules)

These rules are numbered because divergence comments cite them by name.

- **R1 — aggregates.** Selection scans go through
  `lists.min_by`/`max_by`/`min_by_key`/`max_by_key`, never a
  hand-rolled best-so-far loop — EXCEPT hot paths, early-exit scans,
  and allocation-sensitive loops, where the hand scan stays (the
  aggregate takes a closure per element and often needs a materialized
  pair list; a hand scan with two locals allocates nothing). Every
  exception carries an in-place comment naming this rule. Reference
  exemptions: `wfc`'s `lowest_entropy`, `sudoku`'s `best_cell`,
  `checkers`' `best_move`, `swarm`'s `collatz_champion`.
- **R2 — extremes.** Running extremes accumulate with the `min`/`max`
  methods — `worst = worst.max(dr)` — never an `if`-and-assign;
  `Float.min`/`max` exist since v0.8 precisely for this (`spectra`,
  `synthwave`).
- **R3 — std idioms in non-hot code.** Where a std spelling exists, use
  it: `lists.fill`, `pad_left`, `unwrap_or`, `Range.any`/`all`,
  `.all()`/`.any()` over the manual loop/match/gate spellings. This
  applies to ports' INTERNALS too; a port's API *surface* that mirrors
  its upstream is exempt — there the upstream's names and shapes win
  (`ports/*/CONTRACT.md`), because upstream fidelity is the port's
  whole verification story.
- **R4 — deliberate divergence.** Any exemption from a rule in this
  file carries a one-line comment at the site naming the rule it
  diverges from and why, and `NOTES.md` keeps the ledger. An
  uncommented divergence is a bug even when the divergence itself is
  right.
