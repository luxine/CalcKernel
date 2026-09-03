# Implementation blocker 13: hosted-runner descheduling polluted compile-time evidence

Date: 2026-09-04

## Finding

The repaired v0.12 and v0.13 performance runs demonstrated that parent-process wall
time can include large hosted-runner scheduling stalls. Schema 9 used the same clock
for its paired `tuneUse`/`v014Ordinary` and `v014Ordinary`/`v013Ordinary` compile
comparisons, so an otherwise conforming compiler could fail the frozen stability or
compile-regression gates for work it did not perform. V0.14 also pinned the superseded
v0.13 candidate rather than the repaired accepted-base candidate.

## Resolution

- Compile comparison samples now use cumulative `RUSAGE_CHILDREN` user-plus-system
  CPU time before and after each terminated compiler invocation.
- Runtime throughput and standard-session resource limits retain their existing
  monotonic wall-clock protocols; only compile comparison evidence changes clocks.
- The three warmup pairs, fifteen measured pairs, alternating order, upper median,
  corpus, and every threshold remain unchanged.
- The exact v0.13 replay pin advances to
  `d5a2491672477634070b0c36b77cb8ad4bf7df56`; its replay manifest digest is
  `cc33d80608fea92bb90e4c42ebf1977736bf0c251adb3d0f465449b69f87a51e`.

## Regression proof

`schema_nine_compile_comparison_should_measure_terminated_child_cpu_time` fails
against the old collector and requires both the terminated-child clock and its use
by the authoritative compile-comparison path. The ordinary wall-clock resource path
is intentionally unchanged.
