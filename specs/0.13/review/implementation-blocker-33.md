# Implementation blocker 33: steady dispatch null-branch overhead

Date: 2026-09-07

## Finding

Exact V0.13 run `34106156091`, x86-64 performance job `101691953958`, passed
the cumulative schema-7 gate but failed the unchanged schema-8 individual
dispatch/direct ceiling for `trip-unroll-simd`. The multiversion median was
`14,142 ns`; the byte-identical selected hidden member median was `13,272 ns`;
the ratio was `1.06555 > 1.05`. Each channel retained twenty stable samples,
used the unchanged 16-call batch, and recorded exactly one resolver call.

The complete `performance-x86-64` artifact `10015282738` retains the report,
shared objects, profiles, replay bundle, and host diagnostics. Its stripped ELF
shows that the resolved public hot path still executes an atomic slot load, a
null test, a conditional branch, and an indirect jump before the exact selected
member. The selected-direct channel bypasses only this public thunk. On this
compact kernel the redundant null-sentinel branch is large enough to violate the
existing individual overhead budget; the distributions do not support treating
the result as an outlier.

## Rediagnosis

The null sentinel makes every steady call ask whether resolution has happened,
although the slot can encode that state without a branch. Resolution and pointer
publication remain correct if the initial non-null slot value is a cold,
baseline-safe function with the exact public ABI. That entry can resolve and
compare-exchange itself with the selected member, then must-tail-call the result.
Concurrent first callers still compute the same verified selection and observe
either the resolver entry or the one published member.

## Resolution

Each generated slot now starts at a private `resolve_entry` with the exported
function type and calling convention. The public thunk always performs one acquire
load and one indirect must-tail call. On first use that target is `resolve_entry`;
after the resolver's acquire/release compare-exchange it is the selected hidden
member. The steady path therefore has no null comparison or conditional branch,
while the first-call detector, fail-closed baseline choice, concurrent publication,
stable public address, and selected-direct evidence protocol remain intact.

The object-affecting identity advances to `dispatch-resolver-sentinel-v2` in both
the multiversion dispatch identity and code-generation contract, preventing reuse
of old cached objects. A source-level RED/GREEN contract and the Native LLVM thunk
regression require the non-null resolver entry and absence of the old
`ck.dispatch.uninitialized` branch.

## Contract preservation

Language and public ABI, Native ABI 1, Runtime ABI 2, normalized capability rules,
target features, selected variants, correctness and safety semantics, performance
and stability thresholds, batch count, timed work, warmups, samples, corpus,
platform matrix, and required jobs are unchanged.

## Verdict

Accepted implementation blocker. Focused and complete local gates plus a fresh
exact-SHA V0.13 CI run remain mandatory; the x86-64 performance job is the final
authority for the fixed overhead.
