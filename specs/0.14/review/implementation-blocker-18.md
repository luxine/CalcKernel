# Implementation blocker 18: hosted-runner CPU-time sampling and failure evidence

Date: 2026-09-04

## Finding

Exact V0.14 CI run `33808562098`, AArch64 performance job `100825742078`, failed
while rebuilding and checking exact V0.13 revision
`1a7f89b841ce3033063ef4a8ac458aa695c8a8c0`. The inherited schema-7 checker rejected
`vectorSuites/unchecked/slp_quad candidateSamplesNs` as unstable after same-core
affinity and four conditioning batches. The raw log again showed the established
approximately 4.42 ms / 8.84 ms bands.

Same-core affinity rules out cross-vCPU migration as the remaining cause but cannot
prevent a hosted runner from descheduling or throttling the benchmark thread during
a short wall-clock sample. The failed report existed inside the detached replay
checkout under a hidden `.source` directory; the workflow artifact path therefore
did not retain it after the checker exited nonzero.

## Resolution

The shared schema-7 native runtime harness measures the unchanged `invoke_repeated`
kernel-call loop with `CLOCK_THREAD_CPUTIME_ID` on Linux. It retains the existing
single-allowed-CPU scope and four conditioning batches. Current-thread CPU time
counts the benchmark thread's actual execution while excluding intervals in which
the hosted runner does not schedule that thread. Non-Linux hosts retain the existing
`Instant` monotonic-clock implementation.

V0.14 inherits the repaired V0.12/V0.13 harness and regression tests. Every active
V0.12 replay owner pins `ea822e343967baa2db113d3dd8f429d8dfdfa779`; every active
V0.13 replay owner pins `4cbaa0fb970a5ee2112d5d4f54d1a6e0186f875a`;
both manifest digests are recomputed from those exact identities. During historical
schema-8 preparation, the report, evidence tree, retained checker, and replay bundles
are copied into the non-hidden output directory before invoking the checker, so a
rejection is present in the uploaded artifact.

No timed kernel work, conditioning count, sample count, statistic, threshold,
corpus, oracle, optimization/tuning policy, language/ABI rule, or platform/job matrix
changes. V0.12, V0.13, and V0.14 each require a fresh exact-SHA CI run; a later
version cannot sign for an earlier version's gate.
