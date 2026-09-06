# Implementation blocker 23: V0.14 must consume the repaired V0.13 baseline

Date: 2026-09-05

## Finding

Exact V0.12 run `33966418774` failed the unchanged x86 domain-fact gate because
the selected `VF4/UF2` noalias loop serialized each UF chunk as
load/compute/store. V0.13 imported the dependency-ready x86 schedule and
repinned its exact V0.12 replay. V0.14 both inherits this optimizer and builds
an independently pinned V0.13 replay, so the previous V0.13 accepted-base SHA
became stale.

Exact V0.14 run `34008425924` later failed before schema-8 measurement because
the schema-7 vector preflight still required an in-KIR `vector_` operation for
`specialized_length`. The repaired V0.13 compiler intentionally preserves that
constant-bound loop for the Native LLVM loop vectorizer, and records the closed
`constant-call-loop-deferred-to-native-loop-vectorizer` fallback instead. The
benchmark contract had not yet recognized this audited handoff.

## Resolution

V0.14 imports the same x86 `UF > 1` SSA/MemorySSA list scheduler and structural
regression test. It also accepts the Native LLVM handoff only for the exact
`specialized_length` fixture paired with the exact audited fallback reason; all
other vector cases still require materialized KIR vectors. Its accepted-base and
independently built V0.13 replay are
repinned to `f98a7b91e27b09ad2f50a8f4808f183cc87e80fe`, including the recomputed
replay-manifest digest.

No V0.14 tuning policy or artifact format changes. No language/ABI rule,
performance threshold, timed work, sample count, statistic, corpus, CPU policy,
or required platform/job matrix changes.
