# Implementation blocker 55: reuse independently checked multiversion variants

## Evidence

Exact-SHA V0.14 workflow-dispatch run `34198065606`, commit
`68167549ad8ac9ecbbdeb6990313f7106e985e17`, failed required AArch64 performance
job `101970492265` while preparing the accepted V0.13 schema-8 replay. The retained
checker rejected the multiversion source-to-object geometric mean at
`2.500001057595058`, against the unchanged `2.5` maximum.

The complete retained report contained all fifteen ordinary and all fifteen
multiversion samples for each of the five compile cases. Its per-case ratios were
`2.780213568`, `2.721564897`, `2.358156699`, `2.538575052`, and `2.155963681`.
The independently executed V0.13 AArch64 job for the same candidate SHA passed at
`2.35826535437881`, with per-case ratios `2.536077348`, `2.587216828`,
`2.229816264`, `2.384147318`, and `2.091056971`. The retained V0.14 environment
accelerated ordinary compilation more strongly than multiversion compilation,
exposing a real fixed-limit margin issue rather than missing samples or corrupt
evidence.

## Root cause

The V0.13 CLI retains an opaque `CheckedKirMultiversionBundle` only after an
independent checker reconstructs and structurally validates every accepted variant.
Emission nevertheless ran the complete O0 KIR evidence pipeline again for every
checked variant. Native lowering then immediately validates the same KIR/evidence
tuple before LLVM IR construction. The middle validation added no authority, but
its repeated per-variant work consumed the remaining compile-ratio margin.

## Repair

V0.14 imports the exact V0.13 repair at
`6258089cf44ebc317247e3fc38e2c765e424132e`. Checked emission now accepts an
O0-shaped handoff only when the variant is pointer-owned by the opaque checked
bundle. It preserves exact module equality, contract facts, audit state, empty
proof/PGO projection, the independent bundle checker, and the final native-lowering
evidence validation. The raw public emitter and any variant outside the checked
bundle remain fail closed.

The accepted-base manifest, independent replay preparer, structural contracts, and
current normative/task/acceptance documents are repinned to the same V0.13 commit.
The resulting `benches/baselines/v0_13_replay.toml` SHA-256 is
`6a3f2768b56c737d6060d0f7ed03a103ed7570c4064c6cfe9532b2a91d23d230`.

No language or public ABI rule, strict-FP or safety semantic, target ISA,
schema-8/schema-9 shape, performance/stability/artifact-size threshold, timed work,
sample count, corpus, platform, or required job changes.

## Acceptance

The checked-variant ownership regression, V0.13 replay-pin contract, complete local
gates, and replacement exact-SHA ten-job workflow must pass. Both required
performance jobs must execute the unmodified sample protocol and thresholds; V0.14
cannot substitute its cumulative result for the independently retained V0.13 gate.
