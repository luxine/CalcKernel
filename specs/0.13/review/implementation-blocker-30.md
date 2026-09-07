# Implementation blocker 30: optimizer timing headroom

Date: 2026-09-07

## Finding

V0.14 exact replay run `34090234424`, x86-64 performance job
`101642274739`, rebuilt exact V0.13 candidate
`5c6220758718b1ceac8ae32aec80c660d7b67b5e` and failed the unchanged
cumulative schema-7 optimizer gate:

`KIR optimizer exceeds the 3x individual limit`.

The `example-dijkstra` KIR median was 2,531,741 ns against the frozen V0.10
MIR median of 832,254 ns, or about 3.041x. The preceding exact V0.13 job had
passed narrowly at 2,456,276 ns, or about 2.951x. This is insufficient
headroom for the unchanged individual limit; retrying the same implementation
would not close the defect.

No threshold, workload, sample count, statistic, platform, or required job may
change.

## Rediagnosis

Sampling the same Dijkstra KIR O3 pipeline showed that pre-proof CFG
canonicalization repeatedly builds short-lived ordered maps for block lookup,
incoming phi values, and removal masks. These maps are never iterated: they
are used only for exact-key lookup while the transformation itself retains
source block, edge, parameter, and argument order. Ordered tree allocation is
therefore redundant work and provides no determinism property.

## Resolution

Phi pruning now uses function-local hash lookup tables for those three
non-iterated indexes. Live-root traversal remains a `BTreeSet`, all retained
vectors preserve their original order, malformed CFG still fails without
partial mutation, and the existing reordered-storage regression continues to
produce byte-identical KIR.

On the local AArch64 diagnostic workload, 10,000 measured
`example-dijkstra/kir-o3` iterations moved from a 1.340 ms median to 1.242 ms,
about 7.3 percent lower. This diagnostic is not remote acceptance evidence;
the exact x86-64 schema-7 job remains authoritative.

No CK language or public ABI, optimization result, proof contract, safety
rule, target ISA, performance threshold, timed work, sample count, corpus,
platform, or required CI job changed.

## Verdict

Accepted implementation blocker. Focused and complete local verification must
pass before publishing a new exact candidate SHA. V0.14 must pin and rebuild
that final V0.13 SHA before its own cumulative acceptance can count.
