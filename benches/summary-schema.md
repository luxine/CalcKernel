# CalcKernel Benchmark Report Schemas

`cargo bench --bench ckc_perf` writes a general compiler-stage summary and a
strict Native performance report. Their schema versions are independent.

## General benchmark summary — schema 1

The optional general outputs are `build/perf/latest.summary.json` and
`build/perf/latest.summary.md`. JSON contains `schemaVersion: 1`, command,
generation time, host target, warm-up/iteration counts, and results. Each result
records case, compiler task/stage, samples, minimum, median, p95, mean, and
output units. Markdown presents the same values.

## Native runtime and optimizer gate — schema 4

With `--features native-toolchain`, the harness writes
`target/ckc-perf/results.json` with `schemaVersion: 4`. Top-level identity
includes CPU policy, `fastMath: false`, Clang 22.1.8, target/host, warm-up,
sampling/batching/statistic configuration, non-empty `checked` and `unchecked`
suites, a proof-loop comparison, optimizer timing, and `baselineV010`.

Every runtime case records semantic equivalence, compile/cold-run duration,
repeated Native and Clang sample arrays and medians, peak memory, artifact size,
batch iterations, and validated result. Checked and unchecked suites use the
same exact four-case runtime corpus. `optimizerComparisons` uses the exact six
entries from `benches/cases/native-cases.tsv`; omitting a case is a hard failure.

`baselineV010` must identify commit
`df816502876fba41676f9ebc190e4fadd18cd5a5`, compiler `calckernel 0.10.0`, LLVM,
target/CPU/mode, harness/statistics identity, `sourceDigestCount`, and exact
lowercase SHA-256 `sourceDigests` for every runtime and optimizer source. Any
identity, set, or digest mismatch rejects the report.

`scripts/check-native-performance.py` is the normative schema 4 reader. It
requires the exact pinned identities and portable baseline CPU policy,
recomputes each upper median from its stable sample array, and requires at
least 95% Native/Clang geometric-mean throughput,
at most 10% individual Native/Clang regression, at most 3% geometric and 8%
individual 0.11/0.10 runtime regression, at least 97% checked/unchecked
proof-loop throughput, and a median KIR/0.10-MIR optimizer ratio of at most 2x
for the suite and 3x individually.

Changing a field, equivalence rule, source, baseline, statistic, or threshold
requires coordinated review of the harness, checker, guide, tests, and CI. A
threshold must never be changed merely to make a failing candidate pass.
