# Implementation blocker 11: initialization setup was duplicated into hot PGO paths

Date: 2026-09-04

## Finding

Exact V0.13 CI run `33811191360`, x86-64 performance job `100833640363`,
failed the frozen schema-8 generation-overhead gate. For `branch-layout`, the
generation median was 1,075,067 ns versus approximately 135,972 ns ordinary,
or about 7.9 times, above the unchanged 5.0 limit.

## Rediagnosis

The generation object showed that each instrumented function called
`__ck_profile_ensure`, and LLVM then inlined that helper into functions which
were themselves inlined into the kernel loop. Consequently multiple hot sites
materialized the full directory, identity, site-table, and runtime arguments
for `__ck_profile_initialize`. Counter semantics and the benchmark were valid;
the excess work was a compiler-generated code-shape defect.

## Resolution

The LLVM bridge now exposes a guarded `NoInline` function attribute operation,
and KIR profile lowering applies it to the compiler-private
`__ck_profile_ensure` helper. A structural RED/GREEN contract requires the
builder, FFI declaration, C bridge, LLVM `NoInline` attribute, and lowering use.
The initialization state machine is unchanged; instrumented sites perform a
compact helper call instead of cloning the full initialization setup.

An unchanged-protocol local branch-layout measurement after the repair produced
a 431,750 ns generation median versus 282,083 ns ordinary, about 1.53 times.
The instrumentation sites and counters, generation semantics, 5.0 threshold,
batch work, sample counts, rotating order, upper median, corpus, and platform
matrix are unchanged. Fresh exact-SHA x86-64 and AArch64 CI remain the dynamic
authority.
