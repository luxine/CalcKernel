# Implementation Blocker 37: v0.13 replay compile-time headroom

## Finding

The exact-SHA v0.14 x86 performance job rebuilt the accepted v0.13 replay and
measured the Dijkstra KIR optimizer at `2,531,741 ns` against its unchanged
`832,254 ns` baseline. The resulting `3.041x` ratio exceeded the required
individual `3.0x` limit, although the preceding v0.13 run had passed narrowly
at `2.951x`. This was a real headroom failure, not an infrastructure error.

## Diagnosis and repair

Profiling isolated short-lived ordered maps in phi pruning that were populated
only for key lookup. v0.13 blocker 30 replaced those maps with `HashMap` while
retaining the ordered live-value set and without changing KIR semantics,
workload, samples, thresholds, corpus, or required jobs. The repair is the
accepted v0.13 commit
`60e26ac01444903180b90ee3bf7da08c905c0915` and was integrated into v0.14 as
commit `20ea1cf`.

The v0.14 replay manifest and preparation contract are re-pinned to that exact
v0.13 commit. The updated replay manifest SHA-256 is
`a8feea2ad72cfdae9135ff5ba43add8071fbdb7344b3021d2c03aa243aa4eddf`.

## Closure criteria

- the replay preparation script accepts only the repaired exact v0.13 commit
  and the updated manifest digest;
- the contract test enforces both identities;
- the unchanged local performance, contract, feature, lint, release, and audit
  gates pass; and
- new exact-SHA v0.13 and v0.14 workflow-dispatch runs validate the remote
  x86/AArch64 matrices independently.
