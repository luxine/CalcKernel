# Implementation design correction 13: preserve production IR during x86 reduction discovery

## Trigger

The repaired V0.12 exact-SHA run exposed a stable x86-64 regression in
`scalar unchecked/integer_accumulate`: candidate Native was approximately 14.86% slower than the
exact V0.11 replay, exceeding the unchanged 8% limit. Same-worker Clang calibration was stable.

## Correction

The inherited x86 reduction handoff had run `mem2reg` on every production O3 function before
deciding whether the function was a target integer memory reduction. Discovery now runs on a
detached function clone and writes only the accepted interleave metadata to the matching production
loop. Functions without a possible non-local load are skipped before cloning. The fixed interleave
width, KIR fallback policy, semantics, corpus, statistics, and thresholds are unchanged.

V0.14 also advances its exact V0.13 replay pin to
`c44b99cc1954a3ca133cf03c281d0590ce320edb`, which contains the same correction. The updated
`benches/baselines/v0_13_replay.toml` digest is
`a7d5df898bb5fa752dbe8c7f5ac4e46bef3c87d30475d3a919c0ab2a8c9732f1`. No replay adapter is
permitted.

## Acceptance

Local structural, Native, and contract checks establish that analysis no longer mutates production
IR and that the supported Native surface remains correct. Exact x86-64 remote performance CI must
still confirm the dynamic regression is closed; no older run may sign the new candidate revision.
