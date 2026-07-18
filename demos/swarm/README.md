# swarm — a worker-pool job scheduler

Three `crunch_worker.soc` isolates (v0.7 workers: separate VM, heap, and
GC per OS thread) crunch integer jobs — Collatz max-steps champions and
prime counts per block — handed out over a tiny JSON protocol built with
`std.json`, with the pending queue held in a `std.deque`:

```
parent -> worker   {"cmd":"collatz"|"primes","job":id,"lo":lo,"hi":hi}
                   {"cmd":"quit"}                 (shutdown convention)
worker -> parent   {"job":id,"result":r}          (+ "arg" for collatz)
```

Workers never print; results only travel back over the channel, so the
parent alone decides output order and the whole run pins as golden lines
despite true parallelism — determinism by protocol, not by luck.

1. **Static assignment** — job `k` always goes to worker `k % 3` and the
   parent drains one worker at a time (per-worker replies are FIFO), so
   per-job lines, per-worker totals, and the `std.lists.max_by_key` global
   champion are all pinnable.
2. **Dynamic balancing** — the block sizes are deliberately lopsided; each
   worker holds one job in flight and is handed the next from the deque
   the moment it replies. There is no select/poll over handles, so the
   parent collects in rotation — and pins only schedule-independent facts
   (results aggregated by job id, plus the classic total: 1,229 primes
   below 10,000). No line says which worker did what, so a smarter
   scheduler could drop in without re-pinning.
3. **Panic isolation** — a worker spawned with `["fragile"]` panics on its
   first job. The parent sees `recv() -> None`, gets the panic message as
   `Err` from `join()`, watches the dead handle refuse further sends, and
   reassigns the very same job JSON to a fresh worker, which completes it.

## Run it

From the repo root:

```
./target/release/socrates demos/swarm/main.soc   # run the scheduler
./target/release/socrates test demos/swarm         # golden tests
```

`crunch_worker.soc` is guarded by `worker.is_worker()`, so run standalone
it prints nothing and the golden harness passes it through.

Note: this demo found the v0.7 bug where `worker.spawn`'s relative-path
resolution lost the entry script's directory whenever the script had
imports. The bug was fixed in-round, with
`tests/spec/workers/spawn_with_import.soc` as the regression test, so
`main.soc` now spawns `crunch_worker.soc` script-relative directly;
the two-candidate fallback helper it carried while reporting the bug
lives in git history.
