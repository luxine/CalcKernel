# Implementation blocker 16: cumulative V0.12/V0.13 performance repair

> Superseded by `implementation-blocker-17.md`, which records the residual
> scheduler migration and schema-8 retention defects found by the next exact
> V0.13 run and pins the replacement candidates.

Date: 2026-09-04

## Finding

V0.12 exact CI run `33789875090` failed the AArch64 short-SLP stability gate
because one unmeasured conditioning batch did not settle the four-element
`slp_quad` call-bound benchmark before timing. The retained artifact showed the
same approximate `4.42 ms` and `8.84 ms` execution bands in candidate CK,
pinned C, and pinned Rust. V0.12 repaired the common harness at exact commit
`d67468feec71a539bc436cc67c5b28d1395d5133`; V0.13 inherited that repair and
repinned its V0.12 replay at exact commit
`4e43b896a0a7d3befb456b28497bd9e96789b6ea`.

V0.14 inherits the shared vector benchmark and transitively replays both older
candidates. Keeping either superseded replay identity would break cumulative
acceptance even if V0.14's own schema-9 workload passed.

## Resolution

V0.14 carries the identical fixed four-batch unmeasured conditioning ramp and
its structural regression. Its V0.12 replay owners bind
`d67468feec71a539bc436cc67c5b28d1395d5133`; its V0.13 replay owners bind
`4e43b896a0a7d3befb456b28497bd9e96789b6ea`. Both replay-manifest digests are
recomputed from those exact identities.

No timed work, tuning policy, language or ABI rule, performance threshold,
sample count, statistic, corpus, or platform gate changes. V0.14 must pass its
existing cumulative schema-7, schema-8, and schema-9 gates in a fresh exact-SHA
CI run.
