# Implementation design correction 13: preserve production IR during x86 reduction discovery

## Trigger

The repaired V0.12 exact-SHA run exposed a stable x86-64 regression in
`scalar unchecked/integer_accumulate`: candidate Native was approximately 14.86% slower than the
exact V0.11 replay, exceeding the unchanged 8% limit. Same-worker Clang calibration was stable.

## Correction

The inherited x86 reduction handoff had run `mem2reg` on every production O3 function before
deciding whether the function was a target integer memory reduction. Discovery now runs on an
isolated temporary function clone and writes only the accepted interleave metadata to the matching
production loop. The production list is frozen before cloning; each clone remains module-owned
while LLVM analyzes it and is erased after its mappings are consumed. Functions without a possible
non-local load are skipped before cloning. The fixed interleave
width, KIR fallback policy, semantics, corpus, statistics, and thresholds are unchanged.

V0.14 also advances its exact V0.13 replay pin to
`d5a2491672477634070b0c36b77cb8ad4bf7df56`, which contains the complete correction. The updated
`benches/baselines/v0_13_replay.toml` digest is
`cc33d80608fea92bb90e4c42ebf1977736bf0c251adb3d0f465449b69f87a51e`. No replay adapter is
permitted.

## Acceptance

Local structural, Native, and contract checks establish that analysis no longer mutates production
IR and that the supported Native surface remains correct. The first premature-detach implementation
was rejected by exact x86-64 Native CI with `SIGSEGV`; forcing the corrected path locally passes.
Exact x86-64 remote performance CI must still confirm the dynamic regression is closed; no older
run may sign the new candidate revision.
