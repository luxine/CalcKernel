# CalcKernel Benchmark Report Schemas

`cargo bench --bench ckc_perf` writes a general compiler-stage summary and a
strict Native performance report. Their schema versions are independent.

## General benchmark summary — schema 1

The optional general outputs are `build/perf/latest.summary.json` and
`build/perf/latest.summary.md`. JSON contains `schemaVersion: 1`, command,
generation time, host target, warm-up/iteration counts, and results. Each result
records case, compiler task/stage, samples, minimum, median, p95, mean, and
output units. Markdown presents the same values.

## Native runtime and optimizer gate — schema 6

With `--features native-toolchain`, the harness writes
`target/ckc-perf/results.json` with `schemaVersion: 6`. Top-level identity
includes CPU policy, `fastMath: false`, Clang 22.1.8, target/host, warm-up,
sampling/batching/statistic configuration, non-empty `checked` and `unchecked`
suites, a proof-loop comparison, optimizer timing, `baselineV010`, and a verified
`runtimeReplay` identity with compiler/bundle/recipe/artifact hashes and the exact
eight-channel rotating sampling schedule.

Every runtime case records semantic equivalence, compile/cold-run duration,
repeated Native and Clang sample arrays and medians, frozen V0.10 Native and
Clang medians (`v010MedianNs` and `v010ClangMedianNs`) as unchanged historical
provenance, actual replay Native/Clang sample arrays and medians, peak memory,
artifact size/hashes, batch iterations, and validated result. Both versions and
safety modes are sampled in the same process on identical inputs. Checked and unchecked suites use the
same exact four-case runtime corpus. `optimizerComparisons` uses the exact six
entries from `benches/cases/native-cases.tsv`; omitting a case is a hard failure.

`baselineV010` must identify commit
`df816502876fba41676f9ebc190e4fadd18cd5a5`, compiler `calckernel 0.10.0`, LLVM,
target/CPU/mode, harness/statistics identity, `sourceDigestCount`, and exact
lowercase SHA-256 `sourceDigests` for every runtime, optimizer, and frozen V0.10
C-oracle source. Candidate Native code and the frozen oracle are measured on the
same worker alongside the independently built, pinned V0.10 Native artifacts.
Historical ratios do not substitute for those replay samples. The candidate must not regenerate the oracle through its own C
backend, which could mask a shared frontend or KIR regression. Any identity,
set, or digest mismatch rejects the report.

`scripts/check-native-performance.py` is the normative schema 6 reader. It
requires the exact pinned identities and portable baseline CPU policy,
loads the repository schema-2 baseline manifest and rejects any reported
`v010MedianNs` or `v010ClangMedianNs` that differs from its target/mode/case
entry. It validates the replay bundle and actual library files, exact sampling
order, three warm-up rounds, twenty samples, seven calls per sample and a
twenty-million-input batch. It recomputes every upper median from its stable
sample array. The runtime baseline ratio is
`(candidateNative/currentClang)/(replayV010Native/replayClang)`, not a cross-worker
ratio using historical times. It requires at
least 95% Native/Clang geometric-mean throughput,
at most 10% individual Native/Clang regression, at most 3% geometric and 8%
individual Clang-normalized 0.11/0.10 runtime regression, at least 97% checked/unchecked
proof-loop throughput, and a median KIR/0.10-MIR optimizer ratio of at most 2x
for the suite and 3x individually.

Prepare the bundle with `scripts/prepare-performance-replay.py` and select it
with `CKC_V010_RUNTIME_BUNDLE`. Missing, modified or incorrectly identified replay
evidence is an error, never a fallback to frozen numbers. The exact V0.10 source,
four existing adapters and baseline manifest remain unchanged. Schema-5 reports
remain historical evidence but cannot satisfy the schema-6 release gate.

Changing a field, equivalence rule, source, baseline, statistic, or threshold
requires coordinated review of the harness, checker, guide, tests, and CI. A
threshold must never be changed merely to make a failing candidate pass.
