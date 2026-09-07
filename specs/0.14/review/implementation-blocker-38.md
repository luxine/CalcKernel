# Implementation blocker 38: inherited x86 widening-cast frontend budget

Date: 2026-09-07

## Finding

Exact V0.14 run `34095419897`, x86-64 performance job `101658156635`,
independently rebuilt accepted V0.13 commit
`60e26ac01444903180b90ee3bf7da08c905c0915` and failed the unchanged
cumulative schema-7 gate:

`vectorSuites/unchecked/integer_cast is below 90% of its faster SIMD oracle`.

The CK candidate median was `4,247,259 ns`; the faster Rust SIMD oracle median
was `3,703,185 ns`, so CK delivered about `87.19%` of the required oracle
throughput. All twenty retained samples were stable. The complete failed job
log and `performance-x86-64` artifact supplied the preparation log, schema-8
report, nested schema-7 report, KIR, object files, and disassembly inputs.

## Rediagnosis

The CK x86 object selected `VF2/UF4`, expanding four independent five-
instruction `u32 -> f64` conversion chains into a roughly 98-byte hot loop.
The independent Rust oracle used the same semantic two-lane conversion with
two chains and a roughly 52-byte hot loop. The generic target cost accounted
for ideal operation throughput and loop-control amortization but not the x86
frontend footprint of this multi-instruction legalization. Ordinary integer
maps remained throughput-bound and continued to benefit from four chains.

This is an inherited V0.13 optimizer defect, not a V0.14 tuning-policy defect.
It therefore had to be closed independently on V0.13 and then propagated into
V0.14; V0.14 could not substitute for V0.13 acceptance.

## Resolution

V0.13 commit `002100719bdefdabb0fece50a363e1b797c464d2` ranks x86 widening
`u32 -> f64` candidates with more than two chains behind candidates within the
measured frontend budget. `UF4` remains in the closed frontier and is still
materialized and independently checked; it is a non-winner for this operation.
Focused RED/GREEN tests require `VF2/UF2` for the widening cast and retain
`UF4` for ordinary and three-stream integer maps.

V0.14 integrates that exact repair as commit
`7d15816c505e8f6eac43e2a9ad5b65e1ac54508b`. Both ordinary and multiversion
native object cache identities include
`x86-widening-cast-frontend-budget-2-v1`, so older `UF4` artifacts cannot be
spliced into the repaired build. The accepted V0.13 replay is repinned to
`002100719bdefdabb0fece50a363e1b797c464d2`; the resulting
`benches/baselines/v0_13_replay.toml` SHA-256 is
`2304bb4a6dab1ef060b37200063f1d33f34429c6cf03ad08e774110f91604eca`.

No language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, legality, profitability, proof or growth gate,
performance/stability threshold, timed work, sample count, corpus, platform,
or required CI job changed.

## Verdict

Accepted implementation blocker. Complete V0.13 and V0.14 local gates must
pass before replacement exact-SHA workflow runs are authoritative. The new
x86-64 remote performance jobs remain authoritative for machine throughput.
