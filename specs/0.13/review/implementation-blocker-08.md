# Implementation blocker 08: repaired V0.12 short-SLP replay identity

Date: 2026-09-04

## Finding

V0.12 exact CI run `33789875090` exposed common two-band execution-state
instability in the fixed four-element `slp_quad` benchmark on the AArch64
worker. V0.12 repaired the benchmark at exact commit
`d67468feec71a539bc436cc67c5b28d1395d5133` by applying the same fixed
four-batch unmeasured conditioning ramp to candidate CK, pinned C, and pinned
Rust immediately before each timed sample.

V0.13 inherited both the benchmark and an exact V0.12 replay pin. Leaving that
pin on `d83805075b0ac8986c895b7a287c84eac509b7f9` would validate V0.13 against a
superseded V0.12 acceptance candidate and would omit the repaired benchmark
contract from the cumulative branch.

## Resolution

V0.13 carries the identical short-kernel conditioning repair and structural
regression. Every active V0.12 replay owner now pins exact commit
`d67468feec71a539bc436cc67c5b28d1395d5133` and the replay manifest digest is
recomputed from that identity.

No timed work, PGO policy, language or ABI rule, performance threshold, sample
count, statistic, corpus, or platform gate changes. V0.13 must rebuild the new
exact replay and pass its existing schema-7 and schema-8 gates in a fresh
exact-SHA CI run.
