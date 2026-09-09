# Implementation blocker 68: repeated source-checked replay analysis

Exact V0.14 `51315f703c589b6920750c21c3feac6053ebd127` run `34310897935`
passed the independently built V0.13 replay and fresh cumulative schema 7/8 on
both performance workers. Its schema-9 jobs then exposed two separate failures:

- x86 job `102337421400` ran on AMD EPYC 7763, which lacks the required
  x86-64-v4 AVX-512 feature set. The collector checked this only after tuning.
- AArch64 job `102337421470` passed runtime measurements but failed tune-use
  compilation cost. Branch-layout measured 35,247,000 / 18,159,000 ns; the
  constant-length call measured 53,737,000 / 18,657,000 ns. Their searches have
  17 and 26 expansions. The other five ratios were approximately 1.07–1.12.

Full logs and both complete performance artifact closures were retained. The
fixed compile limits remain geomean 1.10 and per-case 1.20. No timing workload,
warmup, sample, corpus, platform, required job, or stability limit is changed.

## Diagnosis and bounded repair

Release-mode phase profiling of both real sources shows the complete frontier
search dominates the extra compile work. Stack sampling identifies repeated
KIR serialization, verified state construction and O3 suffix work. The same
prefix was materialized ten times in a focused regression. After prefix
retention, another test still observed 25 late/finish phase calls for 17 distinct
inputs, and repeated identity construction up to 33 times.

Search-local retention is bound to one immutable source-backed space. It
retains unfinished prefixes, independently checked phase outputs and canonical
state identities. Phase reuse requires equality of the entire input state,
including evidence, allocators and growth accounting. Identity reuse compares
the full module, not merely a digest bucket. Retention admission is bounded by
256 snapshots and 4 MiB of canonical KIR; this is not an RSS claim. Saturation
falls back to complete replay without reducing the search or changing its order.

The final-choice suffix remains distinct from an extendable prefix. Raw plan
application retains independent replay and the original suffix schedule. Tests
compare every expansion, metric, ordering and plan digest with an uncached
search in all three budgets on both problem sources. They also compare complete
states while extending and revisiting prefixes, reject colliding cache buckets
with unequal input, and exercise admission exhaustion.

Explicit replay now prepares the verified pre-tune checkpoint and ABI surface
without an unused ordinary KIR O3 suffix. Ordinary compilation and all existing
inspection reports keep their complete pipelines. The actual selected state is
still independently replayed, verified, lowered, linked and identity-checked.

The hardware attestation is collected immediately after retaining the exact
toolchain, before tuning setup, and reused in the final report. A test observed
the old incorrect ordering before passing with early capability validation.
This diagnoses an unsuitable runner earlier; it does not supply AVX-512 or
relax the required hardware.

## Acceptance boundary

Local semantic regressions are not remote performance acceptance. A real
Darwin standard-budget tuning session produced a valid decision for the
constant-length source. With search retention but before removing the unused
ordinary KIR suffix, the full CLI replay remained 31,119,000 / 14,475,000 ns =
2.1498445596 in one 3-warmup/15-sample rotating cohort. This proves the remaining
compilation-performance blocker is not closed by retention alone. Broader
KIR-state/serialization work requires a separate architectural review; no
further speculative local optimization is included here.

The final prepared-checkpoint release, signed with the actual Darwin runtime
entitlements, measured 38,041,000 / 19,460,000 ns = 1.9548304214 in its own
complete rotating cohort. This is still above the frozen per-case limit; the
different cohorts/signature states are not an isolated speedup claim. The real
replay accepted the complete source/frontier/plan/object identities and emitted
byte-identical dylib and header content to the retained cold decision.

Local verification passed formatting, both all-target Clippy configurations,
the complete default and all-feature test suites, 58 Python performance
contracts, the release build, actual hardened-compiler dependency audit, both
Native fixture-tree audits, and JIT permission audit. The two existing ignored
private child helpers remain unchanged. These checks establish correctness and
audit coverage, not a passing schema-9 compile-performance result.

The V0.13 replay remains pinned to exact
`d85e0c786aaeeaa4dbaab9bffa01fcbd5f7c9f5a`; its independent CI is not replaced
by the V0.14 results. Both branches still require their complete exact-SHA
ten-job acceptance. A guaranteed v4-capable x86 runner remains an infrastructure
dependency; no paid runner was provisioned.
