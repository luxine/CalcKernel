# Implementation blocker 10: AArch64 short-SLP conditioning instability

Date: 2026-09-04

## Finding

Exact candidate CI run `33789875090`, AArch64 performance job
`100764340795`, failed the unchanged schema-7 stability gate for
`vectorSuites/unchecked/slp_quad` in the Rust SIMD channel. Artifact
`9908597032` shows that this was not a CK-only regression: candidate CK, pinned
C, and pinned Rust all entered the same approximate `4.42 ms` and `8.84 ms`
execution bands. Rust alone happened to split enough samples across the bands
to trip the median-stability rejection. The other seven unchecked vector cases
were stable on the same worker.

The four-element SLP case performs five million calls per timed batch. Its
single pre-sample conditioning batch was therefore insufficient on the Azure
AArch64 Neoverse-N2 worker to settle the process into one sustained execution
state before timing. The common two-band behavior across all three independently
built channels rules out a candidate-code-only explanation.

## Resolution

Immediately before every timed `slp_quad` sample, the harness now executes a
fixed ramp of four identical unmeasured batches on the same runner. Candidate
CK, pinned C, and pinned Rust receive the same conditioning. A structural
regression test requires the fixed count and requires the loop to appear
immediately before the timer starts.

This changes no timed work: the 20,000,000 batch iterations, twenty stored
samples, seven timed repetitions, rotating channel order, upper-median
statistic, performance thresholds, corpus, and platform matrix remain
unchanged. A fresh exact-SHA AArch64 performance job is required to prove the
dynamic repair; the failed run cannot sign acceptance.
