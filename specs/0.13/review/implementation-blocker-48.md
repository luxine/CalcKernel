# Implementation blocker 48: discard-free multiversion target materialization

## Evidence

V0.14 run `34281039339`, commit `abad55eff21d0b0fb8dfa3afef8b6e9ef6e80a86`,
replayed exact V0.13 commit `8286b32174e33a4b874d71e8c17a5a605a382f0d` on its
AArch64 performance worker. Its retained schema-8 source-to-object multiversion
geometric mean was `2.5196146028`, exceeding the unchanged `2.5` limit. All five
cases retained three warmups and fifteen samples in all four compile channels.
The same V0.13 commit's independent AArch64 run passed at `2.3144958464`;
different fixed overhead exposed insufficient compile-path margin.

## Root cause and repair

Native target setup constructed a complete synthetic schema-1 target set,
including every tier's fixed cost-query universe, only to discard those
profiles and rebuild every tier from its real LLVM TargetMachine. It also
discarded the already-created baseline TargetMachine and recreated it.

Native setup now reads only the immutable descriptors from the same closed
tier table, materializes each real profile once, and reuses the baseline
TargetMachine. The public synthetic fixture constructor is unchanged. All
materialized tiers still pass the existing profile, target-set and digest
validation; no checker or LLVM evidence validation is removed.

The compile-path contract prevents reintroducing the discarded fixture call.
Native tests compare descriptors against fixtures for six platforms and both
Native consumers, then compare the complete host target set, profiles and
digests with explicit LLVM reconstruction for both consumers. Local native
checks pass; only replacement exact-SHA Linux performance runs can establish
acceptance against the unchanged source-to-object threshold.

## Unchanged acceptance

The schema-8 protocol, all thresholds, sample counts, corpus, CPU tiers,
runtime work, required jobs, output checks and historical failed reports remain
unchanged. V0.14 must repin and rebuild this exact V0.13 revision, not reuse the
previous passing distribution as evidence for the new candidate.
