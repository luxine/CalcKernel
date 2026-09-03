# Implementation blocker 04: compile-time evidence included hosted-worker stalls

## Failure inheritance

Exact V0.12 x86-64 evidence showed that parent wall-clock source-to-object
measurements contain unrelated hosted-worker descheduling. Both the candidate
and exact replay streams had isolated 60 ms to 220 ms values among ordinary
18 ms to 35 ms compiles, causing the unchanged stability checker to reject the
evidence after the actual runtime regression had already been corrected.

V0.13 inherits the schema-7 corpus and used the same wall-clock pattern in its
schema-8 ordinary/PGO/multiversion/combined compile corpus. Leaving either path
unchanged would make release acceptance depend on runner scheduling rather than
compiler work.

## Correction

The inherited Rust harness measures the `RUSAGE_CHILDREN` user-plus-system CPU
delta around each compiler subprocess. The schema-8 Python harness independently
measures the same terminated-child resource delta with `getrusage`. Both paths
remain serial and retain their fixed alternating order.

This is a measurement-validity correction, not a threshold adjustment. Warm-up
counts, fifteen measured samples, upper medians, stability policy, corpora,
cache isolation, individual limits, and geometric-mean limits are unchanged.
Runtime clocks remain monotonic wall clocks because runtime throughput must
include elapsed execution latency.

## Acceptance boundary

Structural contracts forbid wall-clock timing inside both source-to-object build
regions. Performance contracts, schema-7/schema-8 checker regression suites,
formatting, Clippy, and the Native benchmark build must pass locally. Exact
x86-64 and AArch64 CI remain the dynamic authority for stable sample evidence.
