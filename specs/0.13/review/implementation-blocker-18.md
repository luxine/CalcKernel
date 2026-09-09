# Implementation blocker 18: 128-bit SVE needs explicit loop interleave

Date: 2026-09-06

## Finding

V0.14 exact run `34014114894`, job `101435039015`, rebuilt and checked the
exact V0.13 compiler at `f98a7b91e27b09ad2f50a8f4808f183cc87e80fe` on the
Linux AArch64 performance host. Schema 8 failed the unchanged
`dispatchGeoImprovement=1.08` gate.

The retained report and object code showed that the host exposes SVE/SVE2 with
a 128-bit vector length. LLVM's default scalable-vector loop policy emitted
two SVE vectors for the `u32` map/zip loops and one SVE vector for the `f64`
compute loop. This provides the same per-iteration element count and no more
independent work than the Advanced-SIMD baseline. The eligible-suite dispatch
geometric improvement was therefore approximately 1.003 instead of the
required 1.08. Correctness, stability, target selection, and resolver-count
checks all passed; this is a code-generation policy defect rather than a noisy
sample or invalid capability report.

## Resolution

For scalar loops compiled by an AArch64 TargetMachine whose exact feature
string contains `+sve`, the Native O3 bridge now requests a four-way LLVM loop
interleave. Existing loop metadata and already materialized fixed-vector KIR
are left unchanged. A local pinned-Clang experiment confirms that the same
128-bit SVE loop changes from two to four independent scalable-vector
load/compute/store groups, while the exact source contract test freezes the
AArch64-only feature guard and interleave factor.

No performance or stability threshold, timed work, sample count, statistic,
corpus, target tier, resolver policy, or required platform/job matrix changes.
