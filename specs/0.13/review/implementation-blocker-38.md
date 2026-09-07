# Implementation blocker 38: exact multiversion test path and checked constant-map schedule

## Evidence

Exact-SHA workflow-dispatch run `34155662442`, commit
`6fd8234859dfe667419b7be9e601ad79426fd2dd`, exposed two actionable defects:

- Linux ARM64 job `101846838287` and Darwin ARM64 job `101846838311` each failed
  `compact_multiversion_helper_policy_should_survive_o3_before_symbol_stripping`.
  The emitted IR contained neither helper because the test itself ran ordinary O3 before
  constructing the multiversion bundle; ordinary O3 is required to inline both helpers and
  therefore cannot observe the compact multiversion retention policy.
- x86-64 performance job `101846838388` measured checked `specialized_length` at
  `4,860,459 ns` versus the faster `4,052,972 ns` Rust SIMD oracle, below the unchanged
  90% throughput floor. The downloaded candidate object used one scalar element per loop
  iteration, while both C and Rust oracle objects used a bounded two-element schedule.

The x86 samples were stable and used the complete required `20,000,000` batch iterations,
three warmups, twenty rotating samples, and unchanged corpus. The candidate was not a
measurement outlier.

## Root cause and rejected shortcuts

The pre-strip Native regression test called `run_kir_pass_pipeline`, although production
multiversion compilation calls `run_kir_multiversion_pass_pipeline`. The former deliberately
uses the ordinary 32-unit inline budget, so the test destroyed the `cold_step` call before the
lowering policy could mark the retained helper `noinline`.

The checked-map repair from blocker 37 then classified every checked scalar load/store map as
streaming and attached `llvm.loop.unroll.disable`. That is correct for unknown-length public
maps, but it also covered an internal map whose bound argument is proven constant at every
direct call. The later constant-call scheduler correctly refused to overwrite existing loop
metadata, leaving this exact specialization scalar and single-lane.

Changing the performance threshold, reducing samples or timed work, naming the benchmark
fixture in the bridge, enabling unchecked overflow, or restoring harmful expansion for all
checked maps are rejected.

## Repair

The Native regression now enters through the exact multiversion KIR pipeline and first asserts
that KIR retained `cold_step` while inlining `hot_step`; LLVM O3 must then preserve that checked
decision. The x86 handoff recognizes the existing IR-semantic proof that a scalar memory map's
bound parameter is constant at every direct call. A checked constant-call map receives the
existing bounded two-way checked-loop schedule, while an unknown-length checked streaming map
continues to receive `llvm.loop.unroll.disable`. Unchecked constant-call maps retain their
separate 1x5 schedule. Ordinary and multiversion object cache identities advance to
`x86-checked-memory-map-schedule-v2`.

No language or public ABI rule, overflow/bounds behavior, target ISA, inline or growth budget,
performance/stability/artifact-size threshold, timed work, sample count, corpus, platform, or
required job changes.

## Acceptance

The focused structural test must fail before the bridge repair and pass afterward. Formatting,
no-native lint/tests, the optimizer multiversion retention test, and source/provenance checks
are rerun locally. A replacement exact-SHA ten-job run remains authoritative for LLVM Native
IR on all six hosts and for the unchanged x86/AArch64 performance gates.
