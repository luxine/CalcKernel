# Implementation blocker 20: accepted performance bases advanced after exact-CI repair

Date: 2026-09-04

## Finding

Exact V0.13 run `33820321093` failed AArch64 performance job `100861409852`.
The inherited `slp_quad` CK, C, and Rust samples entered the same two-band host
state. Local all-features revalidation of V0.13 also exposed an incomplete
Darwin profile-runtime import description under the current SDK.

## Rediagnosis

The schema-7 32-batch conditioning ramp lived inside each of the seven calls
used for one retained upper-median sample. Each retained sample therefore ran
224 unmeasured batches before selecting its value. Separately, the current
Darwin SDK can lower the V0.13 profile runtime's `fstat` call to the stable
`_fgetattrlist` symbol, which the older closed import description omitted.

V0.14 already superseded the second implementation with a direct,
descriptor-bound `fgetattrlist` identity query and an explicit import. It must
retain that stricter implementation rather than regress to the V0.13 source.

## Resolution

V0.14 inherits exact V0.12 commit
`0de952ba5f17ad353ffb00f59b6349c96568b239` and exact V0.13 commit
`6dba7ada778dab868a8e7c507db9c2c0d85c9749`, with both replay manifests
repinned. The inherited sampler now runs one 32-batch ramp before each retained
seven-call sample and never repeats it inside those timed calls. The V0.14
Darwin profile runtime and its stronger failure-step/identity contracts remain
unchanged.

No timed work, sample count, seven-call count, statistic, stability rule,
performance threshold, corpus, target policy, tuning policy, public ABI, or
required platform/job matrix changed.
