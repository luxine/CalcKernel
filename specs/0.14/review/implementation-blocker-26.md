# Implementation blocker 26: inherited Native handoff evidence and scheduling

Date: 2026-09-06

## Finding

Exact V0.13 run `34021087906` exposed two independent failures after all
preceding correctness and stability checks passed. Its x86-64 performance job
`101453829767` measured unchecked `specialized_length` at 748,836 ns against
the faster 665,913 ns SIMD oracle, below the unchanged 90% individual
throughput floor. Its AArch64 performance job `101453829694` measured
`compute-bound` multiversion at 96,667 ns against 91,618 ns selected-direct,
approximately 1.0551 and above the unchanged 1.05 ceiling. Exact V0.14 run
`34021089423`, replay job `101453852634`, reproduced the AArch64 failure while
building the same pinned V0.13 revision.

## Rediagnosis

Retained x86-64 objects showed that CK processed four XMM vectors per loop,
while both independently pinned SIMD oracles processed five. The loop is an
internal scalar memory map with an integer bound supplied as a constant at
every direct call, so the verified KIR deliberately hands its final vector
width and scheduling to Native LLVM. The failure was a missing schedule at
that audited handoff, not a corpus, oracle, or measurement defect.

Retained AArch64 objects showed the selected-direct function preserving
source-proven `noalias`, read-only, and write-only facts, while the separately
emitted SVE member inserted a runtime pointer-distance check. Physical
multiversion emission had revalidated baseline and variant KIR with no
`ContractFactSet`, silently discarding verified alias and effect evidence
before LLVM lowering.

## Resolution

V0.14 imports the V0.13 repair at
`966d54b075a76f2f493d51cb0764688c2ca85675`. Baseline and every enhanced
multiversion module now require the same verified contract facts during their
independent KIR revalidation; missing facts fail closed. Native fact audit
remains the sole authority for LLVM parameter, alignment, effect, assume, and
alias-scope strengthening.

For x86-64 O3, the Native bridge recognizes the internal scalar memory-map
shape and confirms that the bound argument is constant at every direct call
using analysis-only clones. It then attaches a fixed interleave-1/unroll-5
schedule. It does not name a benchmark and excludes checked-overflow,
reduction, and pre-vectorized loops. Production IR changes only through the
accepted loop metadata, and cache identities record both repairs.

The independently built V0.13 replay is repinned to that exact commit. The
updated `benches/baselines/v0_13_replay.toml` SHA-256 is
`d29ecfde60ef72eb46f51016d9e67d8cd1606bbc206e1a20580dd7cbaf235c62`;
no replay adapter is introduced.

No language or ABI rule, performance or stability threshold, timed work,
batch count, sample count, statistic, corpus, target CPU/features, target
tier, tuning search space, or required CI job was changed.

## Verdict

Accepted inherited implementation blockers. Local structural, Native, cache,
and contract tests must pass before a new exact-SHA V0.14 run. Only that new
run may establish remote x86-64 throughput and AArch64 replay acceptance.
