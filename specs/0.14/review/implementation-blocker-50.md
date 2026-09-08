# Implementation blocker 50: schema-8 replay workspace and compile closure

## Evidence

Exact-SHA V0.14 workflow-dispatch run `34172973863`, commit
`70439ee6e69c439a344e83357ae7cd04d34299e5`, rebuilt exact V0.13
`1aad5bdd964f3afa4b367434c1c3810fb63f8e8f`. Its x86-64 and AArch64 performance
jobs both completed full schema-8 collection and failed the preparation step.
Complete job logs and both performance artifacts were inspected.

On x86-64, the stable `memory-bound` multiversion/ordinary ratio was
`20,203 / 19,381 = 1.04241 > 1.03`, while multiversion/selected-direct was
`20,203 / 20,295 = 0.99547`. On AArch64, the five multiversion/ordinary
source-to-object ratios had geometric mean `2.52084 > 2.5`. All correctness,
stability, feature, size, archive, and replay-identity evidence remained valid.

## Root cause and rejected shortcuts

The schema-8 collector rotated eight channels but gave each channel a private
input/output allocation. Persistent alignment, page-placement, and cache-set
differences therefore contaminated cross-channel memory comparisons. The direct
and dispatched multiversion timings being equal isolates the defect to the
measurement workspace rather than dispatch.

The AArch64 path independently repeated an O0 verification pipeline for the
baseline KIR even though the main O3 compilation had already returned the exact
evidence-verified baseline result. The native lowering boundary still performs
its own fail-closed evidence and bundle checks, so this repetition added no
authority.

Lowering thresholds, removing a target tier, reducing timed work, samples,
corpus, platforms, or required jobs, selectively rerunning stable evidence, or
using V0.14 as a substitute for independent V0.13 acceptance are rejected.

## Repair and replay identity

V0.13 commit `21448738b90ccfd1ea9ab79e9355450ef325769c` was first closed with
failing-then-passing regressions. Its schema-8 `KernelWorkspace` now owns one
input/output allocation shared by every loaded code channel for each record and
timed case. Protocol identity advances to
`rotating-eight-channel-shared-workspace-v2`; collector, checker, fixtures,
contract tests, and schema documentation pin the same identity.

Checked multiversion emission now receives the main compilation's verified
baseline result and reuses it for baseline lowering. The raw public emitter
still independently checks the bundle and builds a verified baseline result;
every enhanced variant retains independent verification and target-specific
lowering.

V0.14 inherited that exact repair in commit
`96774b2e3197e4fa11d9286bc4ed78881557a394` and pins historical replay to V0.13
`21448738b90ccfd1ea9ab79e9355450ef325769c`. The resulting
`benches/baselines/v0_13_replay.toml` SHA-256 is
`8852346e6263c3dcf86a30ef2b0084e029865f9deb0545231f73169f0982b148`.

No language or public ABI rule, strict-FP or safety semantic, target ISA,
multiversion frontier, optimization policy, performance/stability/artifact-size
threshold, timed work, repetition count, sample count, corpus, platform, or
required job changes.

## Acceptance

Both V0.14 regressions were observed RED before inheritance and GREEN afterward.
Formatting, lint, Rust/Python performance tests, Native compilation, release
build, and the complete local test suite are rerun. A replacement exact-SHA
ten-job V0.14 workflow must reconstruct and validate exact V0.13
`21448738b90ccfd1ea9ab79e9355450ef325769c`; no old run, moving branch, or V0.14
artifact may substitute.
