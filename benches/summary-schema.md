# CalcKernel Benchmark Report Schemas

`cargo bench --bench ckc_perf` writes a general compiler-stage summary and,
with `native-toolchain`, a strict Native release report. The two schema numbers
are independent.

## General benchmark summary — schema 1

`build/perf/latest.summary.json` and `build/perf/latest.summary.md` record the
command and `schemaVersion: 1`, generation time, host target,
warm-up/iteration counts, case/task/stage, all samples, minimum, upper median,
p95, mean, and output units.

## Frozen 0.11 compatibility report — schema 6

The replayed 0.11 compiler's historical Native report used `schemaVersion: 6`,
`baselineV010`, `runtimeReplay`, `CKC_V010_RUNTIME_BUNDLE`, checked and unchecked
streams, `sourceDigests`, `v010ClangMedianNs`, and the
`replayV010Native/replayClang` Clang-normalized comparison. These names remain
part of the replay audit boundary, but a schema-6 report cannot satisfy the
current 0.12 release gate.

## Native runtime and optimizer gate — schema 7

`target/ckc-perf/results.json` is a fail-closed `schemaVersion: 7` object for
candidate `0.12.0`. Unknown, missing, duplicate, malformed, non-finite, unstable,
or non-positive fields fail. It pins baseline CPU, strict floating semantics,
Clang 22.1.8, Rust 1.90.0, the canonical Native `KirTargetProfile` digest,
cost/proof/budget schema 1, actual artifacts, source/recipe/component digests,
and the exact schedules described below.

### Scalar regression

`rotating-twelve-channel-v1` runs candidate Native, current frozen Clang C,
replayed 0.11 Native/Clang, and retained 0.10 Native/Clang, each in unchecked and
checked mode. Channel order is the top-level `channelNames` array. There are
three warm-up rows and twenty sample rows; row `r` is every channel starting at
`r % 12`. Each stored sample is the minimum of seven calls over the fixed
20,000,000-item batch, and each stored median is the upper median.

`runtimeReplayV011` pins commit
`80c0acf6bb5d65e4d9d40352b9501ea32b79f43d`; `runtimeReplayV010` pins
`df816502876fba41676f9ebc190e4fadd18cd5a5`. Each contains the exact twelve
metadata fields, manifest SHA-256, and eight checked/unchecked artifacts.
Compiler, manifest, recipe, adapters, LLVM component, source diff, artifact
bytes and hashes are independently revalidated. Symlinks, path escapes,
duplicates, substitutions, and historical-number fallbacks fail.

`measuredArtifacts` retains exactly 32 scalar libraries in the report-relative
fresh evidence directory: candidate/current-Clang/replay-0.11-Clang/
replay-0.10-Clang for four cases and two modes. Replay Native artifacts stay in
their independently built bundles.

The cumulative scalar gates are: at least 95% geometric-mean throughput versus
current Clang, at most 10% individual regression, at most 3% geometric-mean and
8% individual Clang-normalized regression versus each real 0.11 and 0.10
replay, and at least 97% checked/unchecked proof-loop throughput. The retained
KIR/0.10 MIR optimizer gate remains 2x suite median and 3x individual.

### Vector and domain-fact suites

`vectorSuites` covers exactly map, zip, strict `f64`, integer cast, modular
reduction, SLP, runtime no-alias versioning, and fixed-length specialization.
`domainFactSuites` covers no-alias and fixed-length contract advantages. Both
separate checked and unchecked modes and use
`interleaved-upper-median-three-channel-v2` with three warm-up rows and twenty
sample rows. Within each retained row, seven raw rounds rotate candidate/C/Rust,
then retain each channel's upper median. Inputs and batching are identical, and
the report retains the upper median of the twenty stored rows.

Vector channels are candidate/C SIMD/Rust SIMD. Every item must reach 90% of
the faster oracle and the per-mode geometric mean must reach 95%. Domain
channels substitute generic C/Rust, and candidate geometric mean must exceed
the faster generic oracle by 5%. Checked and unchecked result digests must
match. `oracleIdentity` pins the manifest, compiler versions, strict math,
differential audit, and UB audit. `oracleArtifacts` retains all 60 actual
libraries with exact suite/case/mode/channel names, bytes, and SHA-256.
On Linux, every stored runtime duration is the current thread CPU-time delta
around the unchanged native call loop inside its same-core affinity scope;
shared-host descheduling is excluded without removing kernel CPU work.

### Object size and source-to-object time

`artifactSizeComparisons` contains both modes of every vector fixture compiled
from the same source by candidate and replayed 0.11 into relocatable objects.
Aggregate candidate growth is at most 35%; no item exceeds 2.5x.

`compileTimeComparisons` uses fresh output paths, no artifact cache, three
alternating warm-up pairs, fifteen alternating measured pairs, and upper
medians of terminated-child user-plus-system CPU time. Candidate/replayed-0.11
source-to-object time is at most 1.5x in geometric mean and 2x individually.

Prepare both replay bundles with `scripts/prepare-performance-replay.py`, select
them through `CKC_V011_RUNTIME_BUNDLE` and `CKC_V010_RUNTIME_BUNDLE`, build the
candidate release compiler, then run the benchmark and
`scripts/check-native-performance.py`. Changing a source, identity, manifest,
statistic, corpus, exclusion, or threshold is a reviewed contract change; a
failing candidate never authorizes weakening this schema.
