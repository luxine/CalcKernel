# CalcKernel 0.14 Performance Guide

[简体中文](../zh-CN/guides/performance.md)

CalcKernel 0.14 adds fail-closed performance report schema 9 while retaining the
schema-8 cumulative compatibility gate. The current contract uses
`recipe.schema = 2`; revision 1 reports remain readable with their original
semantics. A formal release
requires complete reports from fixed x86-64 and AArch64 workers; a local build
or release-candidate identity does not sign those gates. Measurements bind the
candidate SHA, exact 0.12 replay SHA, LLVM/Clang 22.1.8, Rust 1.90.0, hardware
and capability manifests, compiler/oracle/source/recipe digests, training and
held-out corpora, profile shards/final profile, target sets, variant objects,
artifact bytes, sample order, and every raw sample.

The exact ordinary-regression replay is CalcKernel 0.12 commit
`e1bcea461492a5a2619cdb960ea00dd668847f0a`. Clang and Rust PGO oracles receive
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
`interleaved-upper-median-three-channel-v3`. All three channels use one shared data
workspace so allocation placement cannot masquerade as a code-performance difference. Every retained row interleaves seven
rotations of candidate/C/Rust and stores each channel's upper median. For
`slp_quad` only, the unchanged 16-of-20 stability band is evaluated after
per-row common-mode normalization; throughput still uses raw retained durations.

## Cumulative release gates

- Version regression compares v0.14 ordinary with exact replayed v0.13 ordinary;
  a credible per-workload slowdown may not exceed 3%, and its geometric aggregate
  may not credibly regress. Auto-Tuning compares v0.14 tuned with matching v0.14
  ordinary under the same SHA, safety mode, target, input, and timing rows, with
  the same 3% credible per-workload limit and a hard upper-median geometric parity
  requirement independent of paired-row significance.
- Every selected tuned result proves at least 3% validation gain by upper median
  and 16/20 paired rows, otherwise it falls back to byte-identical ordinary output.
  At least two sealed release-held-out workloads must repeat that 3% gain. The full
  v0.13 PGO channel remains mandatory diagnostic-only evidence; a future PGO +
  Auto-Tuning mode must be gated as no weaker than matching PGO.
- Tuned throughput reaches at least 98% of explicit hand-written C/Rust SIMD
  geometrically and 92% per case. The two declared domain kernels exceed the
  faster generic C/Rust oracle by more than 8% geometrically.
- `--tune-use` compile time is at most 10% slower geometrically and 20% per
  case than 0.14 ordinary build; 0.14 ordinary stays within 3%/8% of v0.13.
  These compile comparisons use terminated-child user-plus-system CPU time,
  excluding hosted-runner descheduling without excluding compiler work.
  Tuned artifacts and the deterministic compiler archive remain within 110%.
- Standard tuning stays within 30 minutes and its declared candidate/resource
  bounds, peak RSS stays within 2x, tuning cache stays within 4 GiB, and two
  empty-cache cold runs plus one locked warm reuse satisfy exact determinism.
- A missing x86-64-v4/AVX-512 or AArch64 SVE2 capability fails closed as an
  actionable runner infrastructure failure, not as a compiler performance
  regression; required platforms are never skipped.
- The independent predicated-update gate requires a single-choice non-baseline
  Loop SIMD decision whose fixed inputs execute the attested vector body. With one
  immutable PGO profile on both channels, sealed `N=1024`
  strict-`f64` Floyd-Warshall must run at least 5% faster with Auto-Tuning than
  PGO-only on each stable Linux host; both streams must also satisfy the same
  16-of-20 stability rule, and validation slowdown may not exceed 2%.
  The closed report and sampling contract is
  [Predicated-Update Performance Contract 1](../../specs/0.14/predicated-update-performance-1.md).

- Ordinary no-PGO 0.13 baseline/native versus exact 0.12 replay: geometric-mean
  slowdown at most 2%, individual slowdown at most 5%.
- PGO use versus matching 0.13 ordinary CPU policy: geometric-mean improvement
  at least 5%, with held-out individual slowdown at most 3%. Generation
  execution is at most 5x ordinary on the fixed instrumentation corpus.
- Eligible multiversion dispatch versus portable baseline: geometric-mean
  improvement at least 8%, individual slowdown at most 3%. Dispatch achieves at
  least 98% of direct calls to the exact resolved hidden member in a separate
  byte-identical artifact and is at most 5% slower per case.
  On ELF, the collector reads the public entry from `.dynsym` and the exact
  published pointer from the private `.ck_dispatch_slot` section, so shipped
  products can omit the full local symbol table without changing this proof.
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
cargo bench --features native-toolchain --bench tune_perf -- \
  --task collect --out target/ckc-perf/v0.14-results.json
python3 scripts/check-native-performance.py target/ckc-perf/v0.14-results.json
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

CI runs same-worker diagnostics after either the exact 0.13 historical replay
preparation fails or the current-candidate performance gate fails. The failed
step selects the cohort explicitly: `bash scripts/diagnose-native-performance.sh
historical-v013` reads the retained schema-8 report under
`CKC_V013_RUNTIME_BUNDLE/schema8`, verifies the recorded schema-7 file's size and
SHA-256, and inspects the historical replay copies. The default `candidate` mode
uses the current reports and replay bundles. Historical diagnostics do not require
a successful 0.13 `replay.tsv` and never fall back to a different cohort. Missing,
corrupt or redirected evidence still fails diagnostics; original failures remain
failed and their logs/artifacts are uploaded. Diagnostic success is not acceptance.

PGO, bounded multiversioning, and explicit offline Auto-Tuning ship in 0.14 only
after these gates pass. Indirect-call promotion, scalable KIR, and adaptive JIT
PGO remain future work.
