# Implementation blocker 13: inherited SLP conditioning ran seven times per sample

Date: 2026-09-04

## Finding

Exact V0.13 run `33820321093` failed AArch64 performance job `100861409852`.
The unchecked `slp_quad` samples split between approximately 4.42 ms and
8.84 ms for candidate CK, pinned C, and pinned Rust, so this was shared host
state rather than a CK code-generation regression.

## Rediagnosis

The inherited schema-7 sample is the upper median of seven timed batches, but
the 32-batch conditioning loop lived inside `measure_once`. It therefore ran
before every timed batch: 224 unmeasured conditioning batches per retained
sample. That repeated burst triggered the hosted AArch64 throttling band the
conditioning protocol was intended to avoid.

## Resolution

V0.13 inherits exact V0.12 commit
`0de952ba5f17ad353ffb00f59b6349c96568b239`. One identical 32-batch ramp now
runs for the selected CK/C/Rust channel before each retained seven-call sample;
the seven timed calls then execute consecutively. The oracle manifest records
the scope as `once-per-retained-sample`, and a structural regression test
rejects conditioning inside `measure_once`.

The exact V0.12 replay pin and manifest digest are updated to this repaired
commit. No timed work, twenty-sample count, seven-call count, rotation,
upper-median statistic, stability rule, performance threshold, corpus, target
policy, or required platform/job matrix changed.
