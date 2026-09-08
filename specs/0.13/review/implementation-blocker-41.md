# Implementation blocker 41: schema-8 workspace bias and repeated baseline verification

## Evidence

Exact-SHA V0.14 replay workflow-dispatch run `34172973863`, commit
`70439ee6e69c439a344e83357ae7cd04d34299e5`, rebuilt exact V0.13
`1aad5bdd964f3afa4b367434c1c3810fb63f8e8f`. Both required performance jobs
completed the full schema-8 measurement before the preparation step failed.
The complete x86-64 and AArch64 failed-job logs and both performance artifacts
were downloaded and inspected.

The stable x86-64 `memory-bound` row recorded:

```text
ordinary        19,381 ns
pgo             19,368 ns
multiversion    20,203 ns
combined        20,073 ns
selected-direct 20,295 ns
multiversion / ordinary = 1.04241 > 1.03
multiversion / selected-direct = 0.99547
```

The AArch64 multiversion/ordinary source-to-object medians remained:

```text
branch-layout        43.767 ms / 15.820 ms = 2.76656
call-constant-length 46.020 ms / 16.473 ms = 2.79366
trip-unroll-simd     51.980 ms / 22.147 ms = 2.34704
memory-bound         59.909 ms / 23.336 ms = 2.56724
compute-bound        53.731 ms / 24.581 ms = 2.18588
geometric mean                               = 2.52084 > 2.5
```

All associated stability, correctness, artifact-size, archive-size, feature,
and replay-identity evidence was valid.

## Root cause and rejected shortcuts

The schema-8 harness loaded each code channel with a separately allocated input
and output buffer. Rotation balanced temporal order but could not balance a
persistent address property: alignment, physical-page placement, and cache-set
placement remained tied to the channel for all twenty samples. The near equality
of multiversion and selected-direct on x86-64 proves that dispatch was not the
source of the measured memory regression; both used different storage from the
ordinary channel.

The AArch64 result independently exposed repeated source-to-object work. Before
multiversion planning, the normal O3 compilation already produced a complete
evidence-verified baseline `KirPassManagerResult`. Native multiversion emission
discarded that authority and ran another O0 verification pipeline over a clone
of the same baseline before lowering. The lowering boundary already performs its
own fail-closed evidence and bundle-identity checks, so this extra pipeline did
not strengthen the artifact.

Lowering either threshold, removing an enhanced tier, reducing timed work,
samples, corpus, or platforms, selectively rerunning stable evidence, accepting
per-channel allocations, or weakening the independent bundle checker are
rejected.

## Repair

Two structural regressions were added first and observed failing. The first
requires schema-8 to separate loaded code from mutable workload storage, pass
one `KernelWorkspace` to every channel for a record, and advance the protocol
identity. The second requires checked native emission to receive and reuse the
already verified baseline result.

`KernelWorkspace` now owns the exact input/output allocation and ABI arguments;
`Kernel` owns only the loaded library, entry pointer, ABI signature, and reference
to that workspace. Differential and timed comparisons therefore invoke every
channel on identical addresses while keeping each code image independent. The
protocol identity is now `rotating-eight-channel-shared-workspace-v2`, and the
collector, checker, checker fixtures, structural contract, and schema
documentation pin the same value.

The CLI now passes `compiled.result` with the opaque independently checked
multiversion bundle into checked native emission. Baseline lowering reuses that
verified result. The raw public emitter remains fail closed: it independently
checks the bundle and constructs a verified baseline result for callers that do
not already hold one. Enhanced variants retain their existing independent O0
verification and target-specific lowering.

This repair changes no language or public ABI rule, safety or strict-FP
semantics, target ISA, multiversion frontier, growth ceiling, profitability
floor, performance or stability threshold, timed work, sample count, corpus,
platform, or required job matrix.

## Verification

Both focused RED/GREEN regressions, the complete schema-8 checker regression
suite, all native multiversion tests, all-target/all-feature compilation,
formatting, lint, release, and repository test gates are rerun locally. The
replacement exact-SHA workflow-dispatch run remains authoritative for the full
pinned two-platform performance measurement.
