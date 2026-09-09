# Implementation blocker 23: compatible variant retention and exact selected-direct evidence

Date: 2026-09-06

## Finding

Exact V0.13 run `34034445336` at
`bd0210b4ce89c5001f46ab2128a8c7a73dc6323a` failed AArch64 performance job
`101489988703`: the `compute-bound` dispatch/direct ratio exceeded the unchanged
5 percent individual ceiling. V0.14 exact replay run `34034844096` at
`331fcc7559330ac02c05d4800ad9ad3888d4c211` independently failed both replay
performance jobs. Its retained schema-8 evidence measured:

- x86-64 `compute-bound`: ordinary 70,633 ns, multiversion 70,943 ns, and
  selected-direct 46,327 ns;
- AArch64 `compute-bound`: ordinary 96,403 ns, multiversion 96,411 ns, and
  selected-direct 91,371 ns, a dispatch/direct ratio of about 1.0552.

The same V0.14 run exposed two structural failures. x86-64 native job
`101491046186` selected the checked strict-f64 plan `(VF, UF) = (2, 2)` while
its regression asserted exactly `(2, 4)`. Native integration job `101491046225`
failed Clippy because AArch64-only test imports were unconditional on x86-64.

All retained performance streams were stable. No evidence supports changing a
performance threshold, workload, sample count, statistic, or required worker.

## Rediagnosis

The x86-64 schema-8 capability manifest exposed baseline and x86-64-v3, but
each eligible multiversion artifact retained only an x86-64-v4 hidden member.
The planner ranked lower predicted cost before compatibility breadth, and the
shared full-root budget retained one enhanced member. The resolver therefore
correctly fell back to baseline on the required v3 worker. A profitable v3
member would remain selectable on both v3 and v4 hosts, so cost-first retention
was inconsistent with the required worker contract when only one member fits.

The `selectedDirect` channel was not the selected member of the multiversion
artifact. It was a separately compiled `--cpu native` artifact. Consequently,
the dispatch/direct comparison mixed variant selection overhead with different
code-generation policy, and could report a failure even when dispatch selected
the only compatible implementation. This contradicted the normative
same-selected-tier requirement.

Finally, exact `(VF, UF) = (2, 4)` was an invalid structural proxy for the
performance requirement. The x86 target profile exposes the closed `UF <= 4`
frontier, but the real target cost selected `UF2`. Retained schema-7 evidence
showed that plan reaching about 99.53 percent of Rust SIMD for strict-f64 and
99.12 percent for integer-cast while exceeding the C SIMD oracle. The production
requirement is therefore the checked profitable multi-chain winner plus the
unchanged performance gates, not a forced losing schedule.

## Resolution

The multiversion planner now ranks candidates that have already passed the
unchanged profitability floors by compatibility breadth first, followed by
predicted dynamic cost, KIR units, tier identity, and root identity. A RED/GREEN
planner test proves that a one-variant full-root budget retains x86-64-v3 rather
than x86-64-v4. The cache codegen contract advances with
`coverage-first-variant-ranking-v1`.

The schema-8 collector now creates `selectedDirect` as a byte-identical copy of
the multiversion shared object. It resolves that copy once, reads the unique
private dispatch slot from the ELF64 symbol table, and binds the exact hidden
member address with the public ABI signature before timing. Synthetic ELF
RED/GREEN tests cover the exact public/slot lookup and fail closed on missing or
ambiguous slots. Dynamic loading, symbol lookup, and resolution remain outside
steady timing.

Real-host vector regressions continue to require the x86 profile to expose the
four-chain frontier and require strict-f64/integer-cast to select checked `VF2`
multi-chain plans. `UF2` or `UF4` may win through real target cost; unchanged
performance gates remain authoritative. The integer-cast regression now binds
its assertion to the actual `map_cast` function rather than accepting an
unrelated loop explanation. AArch64-only test imports are target-gated.

No language or ABI rule, safety or floating policy, target tier or feature set,
profitability floor, performance or stability threshold, timed work,
conditioning, sample count, statistic, corpus, platform, or required CI job
changed.

## Verdict

Accepted implementation blockers. Focused planner, parser, contract, schema,
and native regressions pass locally. Complete local gates and new exact-SHA
x86-64/AArch64 performance runs remain required.
