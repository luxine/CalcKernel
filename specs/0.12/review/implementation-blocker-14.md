# Implementation blocker 14: the four-batch SLP ramp did not reach sustained frequency state

Date: 2026-09-04

## Finding

Exact V0.12 CI run `33810653132`, AArch64 performance job `100831900569`,
failed the unchanged schema-7 stability rule for unchecked `slp_quad`. CK, C,
and Rust all exhibited the same approximately 4.42 ms / 8.84 ms bands. The
behavior remained after four identical unmeasured conditioning batches,
same-core affinity, and current-thread CPU-time measurement.

## Rediagnosis

The matching factor-of-two bands across three independently generated kernels
exclude a CK-only optimizer regression. Thread CPU time excludes periods where
the guest thread is not scheduled, but it does not normalize a hosted AArch64
core's sustained frequency state. Four batches provided only about 18--36 ms
of continuous short-kernel work and did not reliably leave that transient
state before timing. The timed work, sample statistic, and stability checker
remain valid and must not be weakened.

## Resolution

The identical per-channel, per-sample unmeasured ramp is increased from four to
32 batches and is now numerically pinned in the oracle manifest. This supplies
approximately 140--280 ms of continuous work at the observed bands before the
unchanged timed batch. A structural RED/GREEN regression requires the exact
32-batch constant and its placement immediately before the timer.

No timed invocation, input, sample count, seven-call upper median, channel
rotation, stability or performance threshold, oracle implementation, corpus,
or platform gate changed. Fresh exact-SHA AArch64 CI remains the dynamic
authority for confirming that the longer equal ramp reaches sustained state.
