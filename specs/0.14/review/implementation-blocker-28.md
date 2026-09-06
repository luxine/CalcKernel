# Implementation blocker 28: inherited Native scheduling closure

Date: 2026-09-06

## Finding

Exact V0.14 run `34031421321` passed Native integration and the completed Linux
and Darwin host jobs, but both required performance jobs failed while preparing
and checking exact V0.13 replay
`aa155825959e49d61fcea7a953935b179a7a238f`. AArch64 job `101481682918`
measured `compute-bound` multiversion at 96,523 ns against 91,539 ns selected
direct, approximately 1.05445 above the unchanged 1.05 ceiling. x86-64 job
`101481682943` measured unchecked `strict_f64` at 4,399,101 ns against the
faster Rust SIMD oracle at 3,610,911 ns, only about 82.08% of the required
throughput. The retained schema-7 report also showed unchecked `integer_cast`
at 4,344,417 ns against 3,499,612 ns, about 80.55%.

## Rediagnosis

The stable x86 objects show two independent XMM chains in CK versus four in the
faster oracle. The checked KIR frontier already permits `UF=4`; the Native
target profile exposed only LLVM TTI's reported factor of two and therefore
prevented the cost model from considering the supported plan.

The AArch64 selected-direct object uses the intended Neoverse-N2 `dech`/`incb`
schedule, while the generic SVE member uses the generic schedule. Attaching
`tune-cpu` alone did not materialize a per-function subtarget. The function must
also carry its already selected `target-cpu=generic` and exact
`target-features`, as in Clang's equivalent `-mcpu=generic+sve2
-mtune=neoverse-n2` lowering.

## Resolution

V0.14 imports exact V0.13 repair
`bd0210b4ce89c5001f46ab2128a8c7a73dc6323a`. x86 Native profiles expose a
minimum maximum interleave of four, still within the existing closed `UF <= 4`
frontier and all independent candidate checks. Generic AArch64 SVE/SVE2
functions materialize the unchanged generic CPU and exact feature string with
the fixed tuning model, allowing machine scheduling to consume it without
expanding the permitted ISA. Real-target regressions cover both x86 vector
plans and AArch64 attributes/object schedule.

The independently built V0.13 replay and accepted-base references advance to
that exact commit. The updated `benches/baselines/v0_13_replay.toml` SHA-256 is
`925eb2410310f4e3aa31247be37f3576468a12cb2a54a421a64f7ed651304d6d`;
no adapter is introduced.

No language or ABI rule, target tier or feature set, tuning search space,
performance or stability threshold, timed work, conditioning, sample count,
statistic, corpus, platform, or required CI job changed.

## Verdict

Accepted inherited implementation blockers. Local full-feature verification
and replay preparation must pass before restarting V0.14. Only the new exact
V0.14 SHA can establish schema-8 replay and schema-9/Contract-1 performance.
