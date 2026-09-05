# CalcKernel 0.13 Performance Guide

[简体中文](../zh-CN/guides/performance.md)

CalcKernel 0.13 uses fail-closed performance report schema 8. A formal release
requires complete reports from fixed x86-64 and AArch64 workers; a local build
or release-candidate identity does not sign those gates. Measurements bind the
candidate SHA, exact 0.12 replay SHA, LLVM/Clang 22.1.8, Rust 1.90.0, hardware
and capability manifests, compiler/oracle/source/recipe digests, training and
held-out corpora, profile shards/final profile, target sets, variant objects,
artifact bytes, sample order, and every raw sample.

The exact ordinary-regression replay is CalcKernel 0.12 commit
`493d3497d3ec89d5cc168d0e92520339e3bf015a`. Clang and Rust PGO oracles receive
the same training/evaluation split and source-level preconditions as CK, disable
fast math/contraction, and pass differential plus undefined-behavior audits.
Training data is never timed as held-out evidence. Correctness also includes a
separate adversarial corpus.

## Sampling protocol

Every timed channel uses identical source mode, input, batch, process, and CPU
policy. Dynamic loading, symbol lookup, and dispatch resolution occur before
steady-state timing; the report proves that resolver execution happened once.
Channels rotate through fixed warm-up and sample schedules, retain every actual
order/sample, use the upper median, and apply the closed stability rule. A
stability failure invalidates the evidence; it does not authorize arbitrary
reruns or deletion of a case. Missing/unknown/extra/mismatched report fields,
digests, streams, tiers, or capabilities fail the checker.

The cumulative 0.12 vector/domain replay uses
`interleaved-upper-median-three-channel-v2`. Every retained row interleaves seven
rotations of candidate/C/Rust and stores each channel's upper median. For
`slp_quad` only, the unchanged 16-of-20 stability band is evaluated after
per-row common-mode normalization; throughput still uses raw retained durations.

## Cumulative release gates

- Ordinary no-PGO 0.13 baseline/native versus exact 0.12 replay: geometric-mean
  slowdown at most 2%, individual slowdown at most 5%.
- PGO use versus matching 0.13 ordinary CPU policy: geometric-mean improvement
  at least 5%, with held-out individual slowdown at most 3%. Generation
  execution is at most 5x ordinary on the fixed instrumentation corpus.
- Eligible multiversion dispatch versus portable baseline: geometric-mean
  improvement at least 8%, individual slowdown at most 3%. Dispatch achieves at
  least 98% of selected-direct geometric mean and is at most 5% slower per case.
- Combined PGO+multiversion is no more than 2% slower in geometric mean and 5%
  individually than the faster matching PGO-only/multiversion-only channel.
- Combined CK reaches at least 95% of the faster equivalent Clang/Rust PGO
  geometric mean and at least 90% on every accepted kernel.
- PGO/multiversion/combined source-to-object geometric-mean ratios are at most
  1.5x/2.5x/3.5x ordinary and individual ratios at most 2x/3x/4x.
  Artifact aggregate ratios are at most 1.25x/2x/2x and individual ratios at
  most 1.5x/2.5x/2.5x. The distributed `ckc` archive is at most 15% larger than
  exact 0.12. Source-to-object samples use terminated-child user-plus-system CPU
  time, excluding hosted-worker descheduling without removing compiler work.
- All cumulative 0.12 gates remain: Native reaches at least 95% of pinned Clang
  geometric mean, no item is more than 10% slower, checked proof loops reach at
  least 97% of unchecked throughput, vector/domain gates remain, and optimizer
  latency retains the prior 2x suite/3x individual ceilings.

Runtime throughput, generation overhead, source-to-object time, artifact size,
compiler archive size, memory, cold/warm execution, and cache behavior are
separate quantities. No threshold authorizes weaker diagnostics, evaluation
order, modular integer behavior, strict floating semantics, checked first-error
order, print/effect order, semantic MIR, public ABI, or contract domain.

## Commands and evidence

Local schema/checker/correctness checks precede expensive stable-worker runs:

The general harness entry point is `cargo bench --bench ckc_perf`; it writes
`build/perf/latest.summary.json` and `build/perf/latest.summary.md`. Native and
PGO measurements add the feature and task selectors shown below.

```sh
cargo test --locked --test performance -- --nocapture
python3 -m unittest discover -s tests/performance -p '*_test.py'
python3 scripts/prepare-performance-replay.py --baseline 0.12 \
  --out target/performance-runtime-replay-v012
python3 scripts/prepare-performance-replay.py --baseline 0.11 \
  --out target/performance-runtime-replay-v011
python3 scripts/prepare-performance-replay.py --baseline 0.10 \
  --out target/performance-runtime-replay
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
cp target/ckc-perf/results.json target/ckc-perf/results-baseline.json
python3 scripts/check-native-performance.py target/ckc-perf/results-baseline.json
cargo bench --features native-toolchain --bench pgo_perf -- \
  --task collect --out target/ckc-perf/v0.13-results.json
python3 scripts/check-native-performance.py target/ckc-perf/v0.13-results.json
```

The Native commands require the pinned `CKC_LLVM_PREFIX`, `CKC_CLANG_ORACLE`,
`CKC_CANDIDATE_COMPILER`, `CKC_V012_RUNTIME_BUNDLE`,
`CKC_V011_RUNTIME_BUNDLE`, and `CKC_V010_RUNTIME_BUNDLE` paths. The same worker
must produce and check both reports; a copied or cross-worker schema-7 report is
not release evidence.

The report is canonicalized and hashed before the independent checker reads it.
The benchmark cannot declare itself passing. Diagnostics inspect only the actual
report/artifacts and do not rebuild or remeasure a required gate. Changing a
source, corpus, profile, target/capability, oracle precondition, threshold,
statistic, exclusion, or checker is a reviewed contract change.

PGO and bounded multiversioning ship in 0.13 only after these gates pass.
Auto-Tuning remains 0.14; indirect-call promotion, scalable KIR, and adaptive
JIT PGO remain future work.
