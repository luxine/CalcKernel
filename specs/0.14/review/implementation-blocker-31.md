# Implementation blocker 31: inherit the exact V0.13 performance closure

Date: 2026-09-07

## Finding

Exact V0.14 run `34041903921`, AArch64 performance job `101510076457`,
failed while preparing and checking its pinned V0.13 schema-8 replay. The
replayed V0.13 AArch64 report had passed cumulative schema 7 but failed the
unchanged dispatch geometric-improvement gate with `1.01545 < 1.08`.

The source run was V0.13 run `34041456107`. Its x86-64 job
`101509032367` independently failed because unchecked `zip_u32` reached only
about 89.9924% of its faster SIMD oracle. Its AArch64 job `101509032163`
produced the schema-8 dispatch failure reproduced by V0.14. V0.14 therefore
had no independent tuning result to diagnose: its accepted base was already
known to be invalid. No threshold, workload, sample count, statistic, platform,
or required job may change.

## Rediagnosis

The x86 V0.13 Loop-SIMD candidate passed cost and legality but redundant
per-chunk offsets and single-predecessor MemorySSA parameters pushed `VF4/UF4`
beyond the unchanged aggregate two-times KIR growth ceiling. On the AArch64
Neoverse N2 worker, LLVM re-inlined a medium cold branch helper into every
multiversion member and if-converted both paths, suppressing the intended
all-hot dispatch gain on 128-bit SVE2.

These are inherited compiler defects, not V0.14 replay-adapter problems.
Adapting the historical report, reducing work, or weakening schema 8 would
hide the invalid base and is forbidden.

## Resolution

V0.14 imports exact V0.13 commit
`b7da701ff7785c4a687ebfd65a8882b1b2a4eac2`. Independent UF chunks use a
shared vector-width recurrence, single-predecessor vector bodies directly use
dominating MemorySSA versions, and the checker reconstructs every chunk start
and the full backedge. Unprofiled multiversion uses an eight-instruction
pure-helper inline budget while ordinary O3 and PGO-hot limits remain 32 and
48. Native lowering preserves still-called 9-through-32-instruction pure
helpers with `noinline` so LLVM cannot undo the KIR clone policy.

The exact replay pin and compiler identity advance to that commit. The updated
`benches/baselines/v0_13_replay.toml` SHA-256 is
`2bb3430ce53239ab13b91baffb84c3337e2fbe9d9c3b48a34d59f41bb603a8ac`.
No replay adapter is introduced.

No CK language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, profitability or growth budget, performance or stability
threshold, timed work, sample count, corpus, platform, or required CI topology
changed.

## Verdict

Accepted inherited implementation blocker. Local replay identity and complete
V0.14 regression gates must pass before pushing a new candidate. Exact remote
x86-64 and AArch64 performance jobs remain authoritative for both the V0.13
replay and V0.14 schema-9 results.
