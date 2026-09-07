# Implementation blocker 44: inherit the complete v3/v4 retained set

## Trigger

Exact-SHA V0.13 run `34117792378` at
`dd6239e677720845dee874ac3095710941b58d5b` failed x86-64 performance job
`101728726666`. The compute-bound combined artifact reached 24,126 ns while the
Clang and Rust PGO oracles reached 17,797 ns and 17,836 ns. Capability evidence
proved the runner supports x86-64-v4, but disassembly proved CK selected an AVX2
YMM member while the oracles used AVX-512 ZMM.

## Diagnosis and inherited repair

Coverage-first ranking correctly put the compatible v3 member first, but the
baseline-sized additional KIR ledger charged both target-profile views as two
complete bodies. It therefore rejected the profitable v4 member. Reversing the
rank would regress v3 runners, and increasing the logical `2x` ceiling or
weakening any performance requirement is not acceptable.

V0.14 inherits V0.13 commit
`0b2eaa52682d06300a009b2378a9ce00697f93f5` exactly. The planner restores the
common baseline profile and verified source symbol names, compares the complete
normalized KIR bytes, and shares one body charge only when those bytes are
identical. Real instruction, CFG, ABI, or mapping differences still pay full
units. Target profiles, proofs, feature/codegen digests, LLVM modules, objects,
audits, cache identities, and physical artifact-size gates remain separate.

The V0.13 replay manifest now pins that exact commit and compiler identity; its
SHA-256 is
`b2fb99873ac107481bac79529b4ac1bdc5e63abbbd2a8e925d6e9c935c612ffd`.
The preparation script, structural tests, current normative accepted-base
references, and final acceptance checklist use the same immutable identity.
Historical blocker records retain the SHAs they actually diagnosed.

No language or public ABI rule, safety or strict-FP semantic, target ISA,
candidate frontier, profitability floor, performance/stability threshold, timed
work, sample count, corpus, platform, or required job changes.

## Acceptance

The inherited regression, full no-native Rust suite, all 46 independent Python
performance-checker tests, formatting, and warning-denying all-target Clippy are
green in V0.14. The replacement exact-SHA V0.14 workflow remains authoritative
for the pinned native, replay, schema-8, schema-9, and performance matrix.
