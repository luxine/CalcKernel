# Implementation blocker 24: V0.14 must consume the repaired AArch64 SVE baseline

Date: 2026-09-06

## Finding

Exact V0.14 run `34014114894`, AArch64 performance job `101435039015`, rebuilt
the accepted V0.13 compiler at
`f98a7b91e27b09ad2f50a8f4808f183cc87e80fe`. Historical schema 8 failed the
unchanged `dispatchGeoImprovement=1.08` gate: the eligible-suite dispatch
geometric improvement was approximately 1.003.

The retained report and object-code audit showed that this host exposes
SVE/SVE2 with a 128-bit vector length. LLVM's default scalable-vector policy
gave the eligible loops essentially the same element count and independent
work per iteration as the Advanced-SIMD baseline. Correctness, stability,
target selection, and resolver-count checks passed, so the result identifies a
code-generation policy defect rather than measurement noise or an invalid
capability report.

## Resolution

V0.14 imports the V0.13 repair at
`4472330758a71312f9a86b83ab10fdce47791287`: scalar O3 loops compiled by an
AArch64 TargetMachine whose exact feature string contains `+sve` request a
four-way LLVM loop interleave. Existing loop metadata and already materialized
fixed-vector KIR remain unchanged. The accepted-base revision and independently
built V0.13 replay manifest are repinned to that exact commit, including the
recomputed replay-manifest digest.

No V0.14 tuning policy or artifact format changes. No language/ABI rule,
performance threshold, timed work, sample count, statistic, corpus, target
tier, resolver policy, or required platform/job matrix changes.
