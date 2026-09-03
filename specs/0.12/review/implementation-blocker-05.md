# CK 0.12 Implementation Blocker 05: AArch64 Sample Aggregation

## Decision

The AArch64 performance failure in exact-SHA run `33747264932` is a benchmark
aggregation defect. It does not justify changing the CK/C/Rust workloads,
reducing the three warm-up rows or twenty measured rows, weakening the
16-of-20 stability rule, or lowering any performance threshold.

## Rediagnosis

The retained `slp_quad` evidence contains the same near-exact 4.4/8.8 ms
bimodality in independently built CK, C, and Rust artifacts. The benchmark ran
seven equal timed calls for each stored sample but retained only their minimum.
That statistic deliberately selected any rare uninterrupted scheduler window
and amplified a shared runner effect into an unstable stored-sample stream.
The candidate code and both independent oracles therefore cannot be the common
cause.

Each seven-call group now retains its upper median. The existing outer upper
median of twenty stored samples is unchanged. This rejects a minority timing
outlier inside a group while preserving every call, the fixed batch, channel
rotation, fail-fast result validation, all raw work, and every acceptance
threshold. The oracle manifest pins `sample_statistic =
"upper-median-of-seven"`, so an unreviewed return to minimum selection fails
closed.

## TDD closure

Before the implementation change, focused tests failed because the aggregation
API and manifest identity did not exist. The behavioral regression supplies one
fast outlier followed by six representative measurements and requires the
representative upper median; a second regression requires immediate error
propagation without consuming the remaining repetitions. The manifest contract
binds the statistic, and the harness calls that tested implementation.

## Acceptance boundary

- CK language semantics, ABI, generated code, optimizer decisions, input corpus,
  and oracle implementations do not change.
- Every measured channel still runs three warm-ups, twenty stored samples, seven
  timed calls per stored sample, and the fixed 20,000,000-item batch.
- The 80% stability requirement and all per-kernel/geometric-mean performance
  thresholds remain unchanged.
- A new exact-SHA workflow must pass both performance architectures and every
  other required job before CK 0.12 can be accepted.

## Self-review

The repair removes a biased reduction of valid measurements instead of hiding
instability or relaxing acceptance. It is narrow to the vector/domain oracle
suite that produced the failure and leaves the frozen historical scalar replay
protocol untouched.
