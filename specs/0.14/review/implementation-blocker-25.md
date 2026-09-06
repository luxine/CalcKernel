# Implementation blocker 25: batched candidate profiling and deterministic SVE scheduling

Date: 2026-09-06

## Finding

Exact V0.13 run `34017771182`, x86-64 performance job `101444674413`, passed
schema 7 and then failed the unchanged schema-8 `generationOverhead=5.0` gate.
The `branch-layout` generation binary took 774466 ns versus 136345 ns for the
ordinary binary, or approximately 5.68x. Object-code inspection found an atomic
candidate-constant runtime call in every hot-loop iteration; the existing edge
counters were already accumulated locally.

Exact V0.14 run `34017772543`, AArch64 performance job `101444700041`, then
failed the exact V0.13 replay's unchanged 5% selected-dispatch overhead gate.
For `compute-bound`, multiversion took 96347 ns versus 91306 ns for the already
selected direct member, or approximately 1.0552x. Correctness, stability,
capability, selection, resolver, and preceding compatibility checks passed.
Disassembly showed that the generic SVE member retained a generic scheduling
model while the direct native comparison used the runner's Neoverse-N2
scheduling model.

## Resolution

V0.14 imports the V0.13 repair at
`b159ea7588359116bb94396215ff793b69e77235`. Candidate-constant observations
now accumulate in saturating function-local counters and publish each exact
bucket once on every normal or checked-failure exit. The profile sites, bucket
meaning, counter totals, runtime schema, and output format are unchanged.

O3 AArch64 functions whose exact target remains `generic` and whose explicit
feature set contains `+sve` now receive the fixed `neoverse-n2` LLVM
`tune-cpu` scheduling model. The generic target CPU and explicit SVE/SVE2
feature string remain the ISA authority, so this does not enable any additional
instruction-set feature. The multiversion cache contract records the scheduling
policy. The accepted-base revision and independently built V0.13 replay are
repinned to the exact repaired commit, including the recomputed replay-manifest
digest.

No language/ABI rule, performance threshold, timed work, sample count,
statistic, corpus, target tier, resolver policy, or required platform/job matrix
changes.
