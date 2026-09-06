# Implementation blocker 20: constant-map scheduling and multiversion facts

Date: 2026-09-06

## Finding

Exact V0.13 CI run `34021087906` exposed two independent failures after all
preceding correctness, stability, capability, resolver, and cumulative checks
had passed. The x86-64 performance job `101453829767` measured the unchecked
`specialized_length` CK candidate at 748,836 ns against the faster 665,913 ns
SIMD oracle, below the unchanged 90% individual throughput floor. The AArch64
performance job `101453829694` measured `compute-bound` multiversion at 96,667
ns against 91,618 ns selected-direct, a ratio of about 1.0551 and above the
unchanged 1.05 dispatch/direct ceiling. V0.14 replay job `101453852634` rebuilt
the same exact V0.13 SHA and reproduced the AArch64 failure.

## Rediagnosis

The retained x86-64 objects showed equivalent four-lane SIMD arithmetic, but
different fixed-trip schedules: CK processed four XMM vectors per loop while
both pinned hand-SIMD oracles processed five. The CK source already records the
constant call bound and the KIR audit deliberately hands that scalar loop to
Native LLVM. Therefore this was a missing schedule at the audited handoff, not
a workload, oracle, or measurement defect.

The retained AArch64 objects showed a more serious evidence-transfer defect.
The selected-direct function retained the source contract's parameter
`noalias`, read-only, and write-only facts. The separately emitted SVE
multiversion function contained a runtime pointer-distance alias check. During
physical bundle emission, baseline and variant KIR were revalidated with
`None` contract facts even though the logical pre-state had been optimized and
verified with those facts. This silently discarded valid lowering evidence and
forced LLVM to version the loop.

## Resolution

`emit_native_multiversion_objects` now requires the verified
`ContractFactSet`; both baseline and every enhanced variant are revalidated
with that exact set before LLVM lowering. The CLI fails closed if the verified
result does not contain contract facts. Existing Native fact audit remains the
authority for every emitted `noalias`, read/write, alignment, assume, and alias
scope strengthening.

For x86-64 O3, the bridge now recognizes an internal scalar memory-map loop
whose integer loop bound is supplied as a constant at every direct call after
temporary mem2reg analysis. It pins interleave 1 and unroll 5 for that loop,
matching the fixed-trip five-vector schedule without naming a benchmark or
changing checked/reduction/pre-vectorized loops. The detector operates only on
analysis clones; production IR changes only through loop metadata. Both the
single-target run cache and multiversion cache identities record the new
schedule, while the multiversion identity also records contract-fact transfer.

The repair was developed from an observed RED structural contract. Focused
Native multiversion, artifact/cache, and performance-contract tests pass
locally. Exact x86-64 schedule throughput and AArch64 SVE dispatch/direct
throughput remain subject to a new exact-SHA remote run.

No performance or stability threshold, timed work, batch count, sample count,
statistic, corpus, source semantics, target CPU/features, target tier, or
required CI job was changed.

## Verdict

Accepted implementation blockers. The fixes restore already verified semantic
facts across physical multiversion lowering and close a demonstrated Native
LLVM handoff schedule gap without weakening acceptance.
