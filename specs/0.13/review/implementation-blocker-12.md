# Implementation blocker 12: exact V0.12 replay required cross-target performance repairs

Date: 2026-09-04

## Finding

Exact V0.12 run `33810653132` failed both performance jobs. x86-64 did not
meet the unchanged domain-fact >5 percent requirement because two vector chunks
did not amortize explicit KIR loop control. AArch64 separately showed a shared
4.42/8.84 ms `slp_quad` frequency band across CK, C, and Rust after the former
four-batch ramp. V0.13 cannot retain a replay candidate that failed its own
required gates.

## Rediagnosis

The x86 issue is target-specific: AArch64's paired-vector path at the same trip
count was already profitable. The AArch64 issue affects all three independent
channels and is an insufficient unmeasured sustained-state ramp, not permission
to weaken sample stability. Both defects belong to the inherited schema-7
implementation and must be repaired in V0.12 before V0.13 can consume it.

## Resolution

V0.13 inherits exact V0.12 commit `3bb6d97ced97aa04c22de8e22238c69a6e107eb7` file by file. The
proposer and independent checker use an x86-64 four-chunk minimum and preserve
the AArch64 two-chunk minimum. The identical unmeasured `slp_quad` ramp is 32
batches on every channel and is pinned in the oracle manifest.

The V0.13 baseline manifest, preparer, measurement harness, independent checker,
contract tests, and current acceptance documents all pin the new exact commit
and manifest digest. No schema-7/schema-8 performance threshold, timed work,
sample count, channel order, statistic, stability rule, corpus, or platform/job
matrix changed. Fresh exact-SHA V0.12 and V0.13 CI runs remain required.
