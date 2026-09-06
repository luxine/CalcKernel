# Implementation blocker 22: x86 vector-chain exposure and AArch64 tuning materialization

Date: 2026-09-06

## Finding

V0.14 exact run `34031421321` rebuilt exact V0.13 candidate
`aa155825959e49d61fcea7a953935b179a7a238f` and failed both required replay
performance jobs at the unchanged V0.13 gates. AArch64 job `101481682918`
measured `compute-bound` multiversion at 96,523 ns against 91,539 ns for the
selected direct member, a ratio of about 1.05445 above the 1.05 individual
ceiling. x86-64 job `101481682943` measured unchecked `strict_f64` at
4,399,101 ns against the faster Rust SIMD oracle at 3,610,911 ns, only about
82.08% of the required oracle throughput. The retained report also showed the
next unchecked `integer_cast` comparison at 4,344,417 ns against 3,499,612 ns,
about 80.55%, so the repair must close both instances rather than stop after
the checker's first error.

All retained sample arrays were stable. No evidence supports changing the
measurement protocol or treating either result as noise.

## Rediagnosis

On x86-64, retained objects showed that CK's strict-f64 loop used two XMM
vectors per iteration while the faster Rust oracle sustained four. The KIR
vectorizer already has an independently checked, closed `UF <= 4` candidate
frontier, but LLVM target-profile discovery reported a maximum interleave of
two. The checked cost model therefore never saw the supported four-chain plan.
The same missing plan accounts for the unchecked integer-cast schedule.

On AArch64, the selected-direct object used the Neoverse-N2 `dech`/`incb`
schedule, while the generic SVE multiversion member retained a generic
`rdvl`/subtract/add schedule. The bridge attached only `tune-cpu=neoverse-n2`.
Without materialized function-level `target-cpu` and `target-features`, LLVM did
not construct the intended per-function subtarget, so machine scheduling
ignored the tuning attribute. Clang's equivalent `-mcpu=generic+sve2
-mtune=neoverse-n2` IR carries all three attributes.

## Resolution

For x86-64 only, Native target-profile construction exposes a minimum maximum
interleave factor of four. This does not expand the optimizer frontier: four
is the existing hard cap, all candidates still come from the same immutable
pre-state, and the independent legality, profitability, growth, proof, and
budget checks remain authoritative. Real-host tests require the x86 profile
and the strict-f64/integer-cast plans to select four independent f64 vector
chains.

For generic AArch64 SVE/SVE2 multiversion functions, the bridge now
materializes the target's existing `target-cpu=generic` and exact
`target-features` string alongside `tune-cpu=neoverse-n2`. A real-object
AArch64 regression checks all three attributes and requires the resulting
`dech`/`incb` schedule. The CPU, declared ISA features, compatibility predicate,
and feature audit are unchanged; tuning still cannot authorize an undeclared
instruction.

Ordinary and multiversion cache codegen contracts advance to identify the new
x86 frontier exposure and corrected AArch64 tuning materialization.

No language or ABI rule, source safety mode, target tier or feature set,
performance or stability threshold, timed work, conditioning, sample count,
statistic, corpus, platform, or required CI job changed.

## Verdict

Accepted implementation blockers. Structural REDs are closed locally; exact
x86-64 and AArch64 performance remain subject to a new exact-SHA remote run.
