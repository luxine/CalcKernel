# Implementation blocker 19: inherited V0.12 and V0.13 candidates failed exact CI

Date: 2026-09-04

## Finding

Exact V0.12 run `33810653132` failed its cross-target schema-7 performance
jobs, and exact V0.13 run `33811191360` failed the unchanged profile-generation
overhead gate. V0.14 cannot claim cumulative acceptance while replaying either
failed candidate, and V0.14's own gates cannot substitute for the missing
version-specific acceptance.

## Rediagnosis

V0.12 required two independent repairs: x86-64 explicit KIR loops need four
vector chunks to amortize entry, backedge, and epilogue control, while AArch64
retains the proven two-chunk floor; the short SLP oracle also needs an identical
32-batch unmeasured ramp for CK, C, and Rust to reach sustained frequency state.
V0.13 separately allowed LLVM to inline the compiler-private profile
initialization guard into instrumented hot paths, multiplying setup overhead.

## Resolution

V0.14 inherits exact V0.12 commit
`3bb6d97ced97aa04c22de8e22238c69a6e107eb7` and exact V0.13 commit
`b6f9fae81f547152181684cf80a2be53443ba994` file by file. The V0.14-specific
profile-guided profitability bypass remains intact; the target-specific minimum
applies only where static profitability is authoritative. The profile guard is
Native `NoInline`, both replay manifests and their digests are repinned, and the
oracle manifest records the 32-batch conditioning protocol.

No schema-7, schema-8, or schema-9 performance threshold, timed work, sample
count, channel order, statistic, stability rule, corpus, or required platform/job
matrix changed. Fresh exact-SHA V0.12, V0.13, and V0.14 CI runs remain required
independently.
