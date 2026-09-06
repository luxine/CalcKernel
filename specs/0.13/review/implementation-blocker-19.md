# Implementation blocker 19: hot candidate publication and generic SVE scheduling

Date: 2026-09-06

## Finding

Exact V0.13 CI run `34017771182`, x86-64 performance job `101444674413`,
failed the unchanged schema-8 generation-overhead gate. `branch-layout`
generation measured 774,466 ns against 136,345 ns ordinary, about 5.68 times
and above the fixed 5.0 limit. The same candidate was rebuilt by V0.14 exact
replay run `34017772543`; its AArch64 performance job `101444700041` failed
the unchanged compute-bound multiversion/selected-direct individual gate:
96,347 ns versus 91,306 ns, about 1.0552 and above the fixed 1.05 limit.

All surrounding correctness, stability, capability, resolver, cumulative
schema-7, and other schema-8 checks completed before these failures remained
valid.

## Rediagnosis

The retained x86-64 generation object showed one
`__ck_profile_candidate_i64` call per loop element. LLVM also correctly
inlined the small `add_path`/`subtract_path` helpers, whose required function
entry observations account for the remaining per-element profile increment.
Selected CFG edges were already locally batched. Candidate hit/miss is a
closed two-bucket event, so publishing it atomically for every observation was
unnecessary compiler-generated overhead rather than required profile meaning.

The retained AArch64 objects showed four SVE vectors in both compared loops,
so blocker 18's interleave correction was active. The generic multiversion
member nevertheless used generic scheduling (`rdvl`, ordinary `add` pointer
updates, and clustered loads), while the native selected-direct member used
the host's Neoverse-N2 scheduling shape (`dech`/`incb` and interleaved load/
arithmetic issue). A pinned LLVM 22.1.8 cross-target experiment reproduced the
selected-direct loop shape with `tune-cpu=neoverse-n2` while leaving the
declared target CPU and features independent. This is a scheduling-model gap,
not dispatch overhead or an unavailable ISA feature.

## Resolution

Candidate-constant sites now maintain separate saturated function-local hit
and other counters. Function exit publishes each exact bucket once through a
bounded compiler-private bulk helper. The canonical site, observation count,
bucket meaning, saturation behavior, profile format, and final profile-use
decisions are unchanged. A real generation-library test checks exact 2,000/
2,000 hit/other publication, and the structural contract rejects a runtime
candidate call in the hot comparison lowering.

For O3 modules whose exact TargetMachine is AArch64, whose CPU remains
`generic`, and whose explicit feature string contains `+sve`, the bridge now
adds only `tune-cpu=neoverse-n2` to defined functions. It does not set
`target-cpu`, add a target feature, change the runtime compatibility predicate,
or bypass the existing object feature audit. The fixed tuning identity is also
included in the multiversion cache codegen contract.

A same-protocol local AArch64 diagnostic after candidate batching measured
329,375 ns generation versus 423,667 ns ordinary for the frozen
`branch-layout` held-out shape. Cross-host exact-SHA CI remains authoritative
for x86-64 generation overhead and AArch64 dispatch/direct throughput.

No performance or stability threshold, timed work, batch count, sample count,
statistic, corpus, instrumentation site/counter/observation, target tier,
resolver policy, or required platform/job matrix changes.
