# Implementation blocker 30: exact V0.13 replay retained only unreachable variants

Date: 2026-09-06

## Finding

Exact V0.14 run `34038295553` failed both required schema-8 replay performance
jobs while independently rebuilding accepted V0.13 commit
`4a04fb34eb0f1358d0f8fa308f95d031954e72b0`:

- the x86-64 worker exposed the v3 capability tier, but the replay artifact
  retained only the v4 enhanced member and failed the unchanged compute-bound
  oracle gate after dispatch selected baseline;
- the Linux AArch64 worker exposed SVE/SVE2, but a dynamic library had no CK
  executable entry to capture the initial auxiliary vector, so both dispatched
  and selected-direct channels remained at baseline and failed the unchanged
  dispatch-improvement gate.

No evidence supports changing a threshold, workload, sample count, statistic,
target platform, or required CI job.

## Rediagnosis

The prior coverage-first ordering ran after the isolated per-tier profitability
filter. A lower compatible tier predicted no regression but could not reach the
retained-set ordering when a profitable strict superset passed the fixed floor.
The resulting one-variant image was legal but unusable on the required lower
capability worker.

The AArch64 artifact contained valid SVE members. Its freestanding detector was
also correctly fail-closed, but it could obtain HWCAP/HWCAP2 only through the CK
executable startup path. Loading the library from the independent Python harness
never invokes that entry path, making baseline the only legal selection.

## Resolution

V0.14 inherits the V0.13 repair commit
`d2a2e5f9fb7ed0c71b1ded4d3bf8c789b4d774ac` exactly. Root eligibility still
requires at least one enhanced tier to pass the unchanged ten-percent and
two-unit profitability floor. A predicted-nonregressing strict feature subset
of that witness may enter the bounded retained set as a compatibility companion;
it remains independently verified and feature-audited. Compatibility breadth
then makes the member executable on the required lower-tier host even when the
growth budget can retain only one enhanced member.

For Linux AArch64 dynamic libraries, the dispatch runtime preserves startup-stack
auxv as the executable fast path and otherwise reads binary `/proc/self/auxv`
through freestanding direct system calls. Missing or incomplete state still
selects baseline, and no libc, loader, allocator, environment, network, or LLVM
runtime dependency is introduced.

The accepted-base pin, independent replay manifest, preparer, contract test, and
normative acceptance references advance together. The updated replay manifest
SHA-256 is
`b730be3b2d40efd7f35195c54e2472b107ccf87f7e85f3095422609f328ea2c0`.

No CK language, public ABI, strict floating rule, safety rule, target ISA,
performance/stability gate, workload, timed region, sample count, corpus,
platform, or required job topology changed.

## Verdict

Accepted implementation blocker. Local exact-replay reconstruction and all
affected gates must pass before a new exact-SHA V0.14 run. Both stable performance
jobs remain the final authority for the repaired selection paths.
