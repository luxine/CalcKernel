# Implementation blocker 29: inherited exact multiversion selection evidence

Date: 2026-09-06

## Finding

Exact V0.14 run `34034844096` at
`331fcc7559330ac02c05d4800ad9ad3888d4c211` failed both required performance
jobs while rebuilding exact V0.13
`bd0210b4ce89c5001f46ab2128a8c7a73dc6323a`. x86-64 job `101491046077`
measured `compute-bound` ordinary at 70,633 ns, multiversion at 70,943 ns, and
selected-direct at 46,327 ns. AArch64 job `101491046160` measured the same case
at 96,403 ns, 96,411 ns, and 91,371 ns respectively. x86-64 native job
`101491046186` also rejected the real `(VF, UF) = (2, 2)` strict-f64 plan against
an exact `(2, 4)` assertion, while Native integration job `101491046225` failed
Clippy on unconditional AArch64-only test imports.

The retained streams were stable. No evidence supports changing a gate,
workload, sample count, statistic, or required worker.

## Rediagnosis

The x86-64 capability manifest exposed baseline and x86-64-v3, but each
eligible replay artifact retained only an x86-64-v4 member because the planner
ranked lower predicted cost before compatibility breadth and the shared budget
retained one full-root clone. The resolver correctly fell back to baseline.

The old `selectedDirect` channel was independently compiled with `--cpu native`
rather than calling the exact hidden member selected by the measured
multiversion artifact. It therefore compared different code-generation policy,
not only dispatch overhead. Separately, retained schema-7 evidence showed the
checked x86 `VF2/UF2` plan reaching about 99 percent of the Rust SIMD oracle and
exceeding C SIMD, so exact `UF4` was an invalid structural proxy for the
unchanged performance requirement.

## Resolution

V0.14 imports exact V0.13 repair
`4a04fb34eb0f1358d0f8fa308f95d031954e72b0`. Candidates that already pass the
unchanged profitability floors retain wider runtime compatibility first, then
use dynamic cost, size, tier, and root identity. The selected-direct collector
loads a byte-identical copy of the multiversion artifact, resolves it, reads the
unique private ELF64 dispatch slot, and binds the exact selected hidden member
before timing. Missing or ambiguous symbols fail closed.

x86 Native regressions continue to require exposure of the closed `UF <= 4`
frontier and a checked `VF2` multi-chain winner, while real target cost may
select `UF2` or `UF4`; unchanged performance gates remain authoritative.
AArch64-only test imports are target-gated.

The accepted V0.13 base and independent replay advance to that exact commit.
The updated `benches/baselines/v0_13_replay.toml` SHA-256 is
`b713147e369c2c4ebd5debcf61005531e5154961ddf864f14bede8baa8599eca`;
no replay adapter is introduced.

No language or ABI rule, target tier or feature set, tuning search space,
profitability floor, performance or stability threshold, timed work,
conditioning, sample count, statistic, corpus, platform, or required CI job
changed.

## Verdict

Accepted inherited implementation blockers. Local full-feature verification
and exact replay preparation must pass before restarting V0.14. Only a new
exact-SHA V0.14 run can establish schema-8 replay and schema-9/Contract-1
performance.
