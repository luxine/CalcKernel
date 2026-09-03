# Implementation blocker 10: inherited runtime samples included hosted descheduling

Date: 2026-09-04

## Finding and rediagnosis

V0.14 exact run `33808562098`, AArch64 performance job `100825742078`,
failed while preparing exact V0.13 replay
`1a7f89b841ce3033063ef4a8ac458aa695c8a8c0`. Its schema-7 checker rejected
`vectorSuites/unchecked/slp_quad candidateSamplesNs` as unstable despite four
conditioning batches and same-core Linux affinity. This falsifies the prior
cross-vCPU-only diagnosis: a shared hosted runner can still deschedule or
throttle the pinned virtual CPU, and monotonic wall time records that unrelated
interval.

## Resolution

V0.13 inherits the V0.12 `CLOCK_THREAD_CPUTIME_ID` runtime sample boundary and
repins exact V0.12 to `ea822e343967baa2db113d3dd8f429d8dfdfa779` with a
recomputed replay-manifest digest. The unchanged native call loop remains
inside the timer, so all CPU work is counted; only time when the shared host
does not schedule the measurement thread is excluded. Same-core affinity and
four-batch conditioning remain fail-closed.

No timed work, samples, rotation, upper-median or stability statistic,
threshold, corpus, optimizer behavior, language/ABI rule, or platform/job gate
changed. Fresh exact-SHA V0.12 and V0.13 CI runs are required.
