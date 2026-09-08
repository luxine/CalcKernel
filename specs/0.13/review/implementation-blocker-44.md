# Implementation blocker 44: remove duplicate checked-variant validation

## Evidence

V0.14 exact-SHA workflow-dispatch run `34198065606`, commit
`68167549ad8ac9ecbbdeb6990313f7106e985e17`, independently rebuilt accepted
V0.13 commit `528f0734a0c4525a2c84158c4d73067e468f292c` on its AArch64 performance
worker. The retained schema-8 checker rejected the multiversion
source-to-object geometric mean at `2.5000010576`, against the unchanged `2.5`
maximum. All five cases contain the required fifteen ordinary and fifteen
multiversion samples.

The independent V0.13 AArch64 job in run `34196798062` rebuilt the same commit
and passed at `2.3582653544`. In the retained replay, ordinary compilation was
consistently faster while multiversion compilation retained more of its cost;
the complete distributions were tight rather than missing or selectively
sampled. This exposed a real compile-path margin problem rather than evidence
corruption.

## Root cause and rejected shortcuts

The CLI retains an opaque `CheckedKirMultiversionBundle` after the independent
checker has reconstructed and structurally validated every accepted variant.
Native emission nevertheless ran the complete O0 optimization-evidence pass
again for each variant. LLVM lowering then performed the same fail-closed
evidence validation immediately before constructing IR. This middle validation
added no authority, but its per-variant CPU cost left the source-to-object ratio
on the fixed limit.

Raising the `2.5` threshold, reducing samples or corpus, selectively rerunning
the failed report, removing the independent bundle checker, or weakening the
final lowering validation is rejected.

## Repair

A regression contract was added first and observed failing. Checked native
emission now creates an O0-shaped result only for a variant whose address is
owned by the opaque checked bundle. The handoff retains exact module equality,
contract facts, audit identity, empty-generation proofs, and the projected PGO
plan. The independent bundle checker remains unchanged, and LLVM lowering still
calls `validate_kir_optimization_evidence` immediately before IR construction.

Raw public emission still reconstructs the checked bundle and baseline O0
pipeline. No language or public ABI rule, safety or strict-FP semantic, target
ISA, output artifact, profile/cache schema, performance/stability/size
threshold, timed work, sample count, corpus, platform, or required job changes.

## Acceptance

The focused RED/GREEN contract, native multiversion suite, formatting, lint,
complete locked tests, release native build, oracle audits, and replacement
exact-SHA ten-job workflow must pass. Both performance jobs must execute the
unchanged schema-8 checker and sample protocol.
