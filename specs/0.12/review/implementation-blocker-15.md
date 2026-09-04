# Implementation blocker 15: conditioning was multiplied inside each retained sample

Date: 2026-09-04

## Finding

Exact V0.13 run `33820321093`, which replayed the repaired V0.12 schema-7
harness, failed AArch64 job `100861409852`. The unchecked `slp_quad` samples
split between approximately 4.42 ms and 8.84 ms for candidate CK, pinned C, and
pinned Rust. Thirty-two conditioning batches therefore did not close the shared
host band.

## Rediagnosis

Each retained sample is the upper median of seven timed batches. The
conditioning loop lived inside `measure_once`, so it ran before every one of
those seven batches: 224 unmeasured batches per retained sample rather than the
specified 32. That long repeated burst triggers hosted AArch64 throttling and
explains why all three independent channels enter the same exact two-band state.
It is not a CK code-generation regression and does not justify relaxing the
stability rule.

## Resolution

The identical 32-batch ramp now runs once for the selected channel immediately
before each retained seven-call sample. The seven timed batches then run
consecutively and retain their upper-median statistic. The pinned manifest names
this scope explicitly as `once-per-retained-sample`, and a structural regression
test rejects conditioning inside `measure_once`.

No timed work, twenty-sample count, seven-call count, rotation, upper-median
statistic, stability rule, performance threshold, corpus, target policy, or
required platform/job matrix changed.
