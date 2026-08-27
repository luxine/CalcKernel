# CalcKernel Benchmark Report Schemas

`cargo bench --bench ckc_perf` writes two kinds of report. They have independent
schema versions because one describes compiler-stage measurements and the other
is the strict Native-versus-C performance gate.

## General benchmark summary — schema 1

The general outputs are:

- `build/perf/latest.summary.json`
- `build/perf/latest.summary.md`

The JSON top level contains `schemaVersion: 1`, the command, generation time,
host target, measured iteration and warm-up counts, and `results`. Each result
records the case, compiler task and stage, sample count, nanosecond minimum,
median, p95 and mean, plus `outputUnits`. The Markdown file presents the same
measurements for humans. These outputs may be redirected with `--out-dir`.

## Native runtime gate — schema 2

With `--features native-toolchain`, the harness also writes
`target/ckc-perf/results.json`. Its top level contains:

- `schemaVersion: 2`;
- `cpuPolicy`, either `baseline` or `native`;
- `fastMath: false` and `clangVersion: "22.1.8"`;
- positive `warmup` and `sampleRepetitions` counts;
- separate non-empty `checked` and `unchecked` suites.

Every case records `name`, `referenceEquivalent`, Native and Clang C compile and
cold-run durations, both repeated sample arrays and medians, peak memory, both
artifact sizes, batched iteration count, and the validated result. Checked and
unchecked suites contain the same case names.

`scripts/check-native-performance.py` is the normative reader for schema 2. It
requires at least 80% of each sample set to lie within 25% of its median, a
Native/Clang geometric-mean throughput ratio of at least 95%, and no individual
Native case more than 10% slower than the equivalent strict Clang C O3 case.
Changing a field, equivalence rule, or threshold requires coordinated changes to
the harness, checker, performance guide, tests, and CI.
