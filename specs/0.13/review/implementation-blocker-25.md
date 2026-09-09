# Implementation blocker 25: compact four-chain Loop SIMD materialization

Date: 2026-09-07

## Finding

Exact V0.13 run `34041456107`, x86-64 performance job `101509032367`,
failed the unchanged cumulative schema-7 gate:

`vectorSuites/unchecked/zip_u32 is below 90% of its faster SIMD oracle`.

The candidate median was 1,493,998 ns, the C SIMD median was 1,594,488 ns,
and the faster Rust SIMD median was 1,344,485 ns. Candidate throughput was
therefore about 89.9924% of the faster oracle. All twenty retained candidate
and Rust samples formed stable distributions, so rerunning without a code
change would not close the defect. No threshold, workload, sample count,
statistic, platform, or required job may change.

## Rediagnosis

The x86 objects use the same 128-bit integer load/add/store operation family.
The CK main loop materialized `VF4/UF2` and handled eight elements per branch;
the Rust SIMD oracle sustained four independent 128-bit chains and handled
sixteen. CK's target profile already exposes the closed `UF <= 4` frontier and
the `VF4/UF4` plan passes target legality and the checked cost comparison.

The plan was rejected only after materialization. The 42-unit source became a
90-unit KIR function, six units beyond the unchanged aggregate two-times
module ceiling. Those six units were representational overhead, not useful
work: every UF chunk rebuilt an absolute induction offset, and the vector body
introduced MemorySSA block parameters even though its sole predecessor's
versions dominate every use.

## Resolution

Loop-SIMD materialization now builds chunk starts as one vector-width constant
plus a verified recurrence. The final recurrence value is the exact
`VF * UF` backedge advance. A single-predecessor vector body consumes the
dominating header MemorySSA versions directly while the backedge still carries
the exact post-store versions. The independent checker reconstructs every
stride edge, confirms the vector-body induction originates at the header, and
continues to validate memory mappings, proofs, cost, structural growth, and
budget charges without calling the proposer.

A RED/GREEN optimizer regression uses a generic three-stream noalias integer
loop rather than a fixture name. It first demonstrated that `VF4/UF4` was
rejected and `VF4/UF2` selected, then proves the compact representation selects
four chains within the original aggregate `2x` KIR ceiling. The object-affecting
cache identity adds `compact-vector-uf-stride-v1`.

No CK language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, profitability floor, code-growth budget, performance or
stability threshold, timed work, sample count, corpus, platform, or required
CI job changed.

## Verdict

Accepted implementation blocker. Focused and complete local verification must
pass before pushing a new exact candidate SHA. The real x86-64 performance job
remains the authority for machine-code shape and throughput, and every other
required job remains mandatory.
