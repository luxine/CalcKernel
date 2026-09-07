# Implementation blocker 31: x86 widening-cast frontend budget

Date: 2026-09-07

## Finding

Exact V0.14 run `34095419897`, x86-64 performance job `101658156635`,
rebuilt exact V0.13 commit `60e26ac01444903180b90ee3bf7da08c905c0915`
and failed the unchanged cumulative schema-7 gate:

`vectorSuites/unchecked/integer_cast is below 90% of its faster SIMD oracle`.

The CK candidate median was `4,247,259 ns`; the faster Rust SIMD oracle median
was `3,703,185 ns`, so CK delivered about `87.19%` of the required oracle
throughput. All twenty retained samples in both channels were stable. No
evidence supports changing the threshold, workload, sampling, or oracle.

## Rediagnosis

The retained x86 object showed that CK selected `VF2/UF4` and emitted four
independent five-instruction conversion chains in a roughly 98-byte loop body.
The faster oracle used the same semantic two-lane unsigned-integer-to-double
expansion with `UF2`, keeping its hot body near 52 bytes. The conversion
expansion is frontend-bound on the stable worker: duplicating beyond two
chains increases instruction footprint without improving its conversion
throughput. Ordinary integer maps remain throughput-bound and still benefit
from the existing four-chain schedule.

The generic target cost ranked `UF4` only from ideal operation throughput and
loop-control amortization. It did not encode the x86 frontend budget consumed
by the multi-instruction `u32 -> f64` legalization sequence.

## Resolution

The Native vector-frontier rank now treats more than two independent chains
of an x86 widening `u32 -> f64` vector cast as exceeding that target-specific
frontend budget. The `UF4` candidate is still discovered, materialized, and
independently checked; it becomes a non-winner rather than being removed from
the closed `UF <= 4` frontier. `VF2/UF2` is selected for this semantic
operation. Non-cast maps and three-stream integer maps retain their existing
four-chain selection.

A RED/GREEN regression constructs an x86 target profile and requires the
widening cast to select `VF2/UF2`; adjacent regressions continue to require
`VF4/UF4` for the standalone and three-stream integer maps.

Because the selected KIR changes native object bytes, both ordinary and
multiversion native cache identities now include
`x86-widening-cast-frontend-budget-2-v1`. A second RED/GREEN contract test
requires the identity in both object-producing paths, preventing an older
`UF4` object from being spliced into a repaired build.

No language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, legality, profitability, proof or growth gate,
performance/stability threshold, timed work, sample count, corpus, platform,
or required CI job changed.

## Verdict

Accepted implementation blocker. Focused and complete local verification must
pass before publishing a replacement exact candidate. The new x86-64 remote
performance run remains authoritative for machine-code throughput.
