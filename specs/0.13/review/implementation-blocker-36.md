# Implementation blocker 36: retained-pair compile cost and runtime priority

## Evidence

Exact-SHA V0.14 replay workflow-dispatch run `34123758500`, commit
`02dcd8a3b1afceac01c5f104d33b0eaaa034d632`, rebuilt exact V0.13
`0b2eaa52682d06300a009b2378a9ce00697f93f5` and failed AArch64 performance
job `101747821716` in `Prepare exact 0.13 compiler and historical schema-8
replay`. The complete failed-job log and every artifact available from the run
were downloaded before diagnosis. The schema-8 report recorded these unchanged
multiversion/ordinary source-to-object median ratios:

```text
branch-layout       47.909 ms / 17.366 ms = 2.7588
call-constant-length 49.512 ms / 18.001 ms = 2.7505
trip-unroll-simd    53.332 ms / 22.710 ms = 2.3484
memory-bound        62.537 ms / 24.294 ms = 2.5742
compute-bound       58.615 ms / 26.219 ms = 2.2356
geometric mean                              = 2.5245647 > 2.5
```

Artifact-size gates remained within their existing limits. The additional
retained target-neutral member exposed enough repeated planner/checker work to
move the aggregate compile result just above the frozen threshold.

## Root cause and rejected shortcuts

The source-to-object path serialized complete normalized KIR modules merely to
compare their target-neutral bodies. It then independently reconstructed the
same checked bundle once in the CLI and again in the raw native emitter. These
operations do not add target isolation or proof strength after the first
successful independent check.

Re-diagnosis also found that the coverage-first retained-set order was copied
unchanged into the runtime dispatch plan. Since the resolver deliberately
selects the first compatible member, a v4/SVE2 host could select its v3/SVE
companion even when the more profitable member was retained. This is an
ordering-authority defect, not a resolver defect.

Lowering the `2.5x` compile gate, removing the required SVE2 member, reducing
timed work or samples, retrying a stable failure, or weakening the independent
checker are rejected.

## Repair

A structural regression test first required the source-to-object implementation
to avoid KIR printing, retain one checker authority through native emission, and
keep the raw public emitter fail closed; it failed before the implementation
changed. The existing retained-pair test was then extended to require runtime
order `[x86-64-v4, x86-64-v3, baseline]`; it failed with the retained storage
order `[x86-64-v3, x86-64-v4, baseline]`.

Normalized target-neutral bodies now compare the complete `KirModule` structure
directly instead of printing it. A successful independent check returns an
opaque authority borrowing the exact immutable request and proposal. The CLI
passes that authority through the source-to-object emission stage, avoiding a
second reconstruction; callers of the raw public emitter still receive the same
independent fail-closed check.

Retention remains coverage-first under the unchanged shared growth budget.
Dispatch construction separately sorts retained members by predicted dynamic
cost, compatibility breadth, KIR size, tier identity, and root identity before
appending baseline. High-tier hosts therefore choose the fastest compatible
retained member while lower-tier hosts skip unsupported entries and retain their
coverage companion. Cache identity advances with
`performance-first-dispatch-ranking-v1` in addition to the existing
`shared-target-neutral-variant-budget-v1` contract.

This repair changes no language or public ABI rule, safety or strict-FP
semantics, target ISA, candidate frontier, growth ceiling, profitability floor,
performance or stability threshold, timed work, sample count, corpus, platform,
or required job matrix.

## Verification

The two focused RED/GREEN regressions are green. Complete local no-native Rust,
Python performance-checker, formatting, lint, release, and smoke evidence is
rerun for the candidate commit. The replacement exact-SHA workflow-dispatch run
provides authoritative native and performance verification on the pinned
runner/toolchain matrix.
