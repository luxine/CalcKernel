# Implementation blocker 13: short-loop SIMD control was not amortized

Date: 2026-09-04

## Finding

Exact V0.12 CI run `33810653132`, x86-64 performance job `100831900623`,
failed the frozen schema-7 domain-fact requirement because the unchecked suite
did not exceed the faster generic C/Rust oracles by 5 percent. The report kept
all required samples and showed `contract_noalias` at 5,275,190 ns versus the
4,842,166 ns C oracle, while `contract_fixed_length` was faster at 4,398,037 ns.

## Rediagnosis

The `contract_noalias` workload has trip count 16. Object disassembly showed
that the accepted VF4/UF2 KIR plan entered its explicit vector loop after only
two complete vector chunks, then also carried setup, backedge, and scalar
epilogue control. The O2 scalar KIR path allowed LLVM to emit a simpler
auto-vectorized loop. The existing abstract break-even calculation modelled
operation throughput but was too optimistic about the concrete control needed
by the explicit KIR loop. This is an optimizer cost-model defect, not unstable
sampling and not an oracle or threshold defect.

## Resolution

Unknown-trip Loop SIMD candidates now use a target-specific floor. x86-64
requires at least four complete vector chunks, `minimum_trip >= 4 * VF * UF`;
AArch64 retains its measured-profitable two-chunk floor. The proposer and the
independent checker derive the floor separately from target identity and the
target-bounded VF/UF. A structural RED/GREEN regression enumerates candidates
for both targets and rejects a candidate below its applicable floor.

The failing x86 VF4/UF2 case now uses a threshold of at least 32, so trip 16
retains the scalar path that LLVM can lower efficiently. Applying that penalty
to AArch64 was rejected during local cross-target revalidation because its
paired-vector path at trip 16 already exceeds the generic oracle; its threshold
therefore remains 16. The 20 percent vector
profitability threshold, schema-7 domain >5 percent gate, sources, oracle
preconditions, timed work, sample counts, channel rotation, upper median,
stability rule, and platform matrix are unchanged. Fresh exact-SHA x86-64 and
AArch64 CI remain the dynamic authority.
