# Implementation blocker 23: V0.14 must consume the repaired V0.13 baseline

Date: 2026-09-05

## Finding

Exact V0.12 run `33966418774` failed the unchanged x86 domain-fact gate because
the selected `VF4/UF2` noalias loop serialized each UF chunk as
load/compute/store. V0.13 imported the dependency-ready x86 schedule and
repinned its exact V0.12 replay. V0.14 both inherits this optimizer and builds
an independently pinned V0.13 replay, so the previous V0.13 accepted-base SHA
became stale.

## Resolution

V0.14 imports the same x86 `UF > 1` SSA/MemorySSA list scheduler and structural
regression test. Its accepted-base and independently built V0.13 replay are
repinned to `795013c6bb1c327395ad7d4c2b71f437d0413e7a`, including the recomputed
replay-manifest digest.

No V0.14 tuning policy or artifact format changes. No language/ABI rule,
performance threshold, timed work, sample count, statistic, corpus, CPU policy,
or required platform/job matrix changes.
