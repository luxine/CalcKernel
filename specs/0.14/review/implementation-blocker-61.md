# Implementation blocker 61: preserve vectorization for fixed AArch64 maps

## Evidence

Exact-SHA V0.14 run `34258812502`, commit
`cd5f463fd9a717d782caa6686269c9a942022616`, completed schema 8 and the full
schema-9 collection in AArch64 performance job `102171708973`. The unchanged
domain-throughput gate then failed. Artifact `10074660702` records these
median times:

- `contract-fixed-length`: generic C `84,473,908 ns`, generic Rust
  `94,187,311 ns`, and CK ordinary/tuned fallback `89,046,408 ns`;
- `contract-noalias`: generic C `80,590,565 ns`, generic Rust `80,627,685 ns`,
  and CK ordinary/tuned fallback `79,159,708 ns`.

The runner reported AArch64 SVE2 and completed the required capability checks,
so this is not a runner capability or infrastructure failure. The tuning
decision for `contract-fixed-length` was `no-candidate`; its tuned artifact is
byte-identical to ordinary, as required by the fallback contract.

## Root cause and rejected shortcuts

The source contract proves `n == 16`, bounds, and no-alias access. Disassembly
of the CK artifact shows eight scalar load pairs, sixteen scalar adds, and eight
scalar store pairs. The generic C artifact keeps a dynamic loop and LLVM emits
an SVE vector loop. KIR discovery correctly defers this scalar loop to Native
LLVM, but the existing handoff supplies only four-way interleave metadata.
LLVM 22 therefore fully scalar-unrolls the short known-bound loop before useful
vectorization.

Lowering the 108/100 domain threshold, widening statistical tolerance, changing
measurements, removing a workload, reducing timed work, or treating this as an
infrastructure failure is rejected. Comparing V0.14 tuned against V0.13 PGO is
not involved: the revised schema-9 like-for-like gates completed independently,
and this failure is the retained domain-throughput requirement.

## Repair

A structural contract was added first and observed failing. The Native bridge
now recognizes an AArch64 `+sve` 32-bit integer scalar memory map whose bound argument has a
proven constant equality. When its trip count is at least 16 and divisible by
the fixed four-lane, four-way-interleave chunk, the bridge attaches
`llvm.loop.vectorize.width = 4`, `llvm.loop.vectorize.enable = true`,
`llvm.loop.interleave.count = 4`, and `llvm.loop.unroll.disable`. The selection
uses cloned analysis IR and verified structure, never a benchmark or function
name. It runs before the general SVE interleave handoff, which remains unchanged
for dynamic loops. LLVM still performs its normal legality checks.

Both ordinary and multiversion Native object-cache identities include
`aarch64-sve-i32-fixed-map-width-4-v1`, preventing reuse of artifacts compiled under
the old schedule.

This changes no language or public ABI, strict-FP or safety semantics, target
eligibility, schema 8 or 9 rule, tuning comparison, performance/stability/size
threshold, timed work, warmup, sample count, corpus, platform, or required job.

## Acceptance

The RED/GREEN structural contract must verify semantic selection, the complete
metadata tuple, cache identity, and ordering before the general SVE handoff. An
AArch64-only target test must construct a generic SVE2 target, compile the frozen
fixed-length CK fixture through the real audited O3 bridge, and observe fixed
four-lane integer vector IR. A direct LLVM 22 C++ syntax check, formatting, lint,
complete locked tests, performance/checker contracts, documentation contracts,
and a replacement exact-SHA V0.14 ten-job workflow must pass. The authoritative
AArch64 performance job must rebuild and remeasure the unchanged workload; no
synthetic local timing may replace it.
