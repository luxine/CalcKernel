# CalcKernel 0.12 Performance Guide

[简体中文](../zh-CN/guides/performance.md)

CalcKernel 0.12 uses strict performance report schema 7. Release measurements
run portable baseline artifacts on stable x86-64 and AArch64 workers, preserve
strict floating-point and safety semantics, and pin compiler, source, target,
canonical `KirTargetProfile`, cost/proof schemas, optimizer budgets, CK/oracle
artifacts, sampling schedule, and every source digest. Native-policy results are
diagnostic unless a separately reviewed hardware identity is frozen.

The scalar regression baseline is the independently built 0.11.0 compiler at
commit `80c0acf6bb5d65e4d9d40352b9501ea32b79f43d`. Its compiler, Native artifacts,
independent C oracle, recipe, and digests are retained with the existing 0.10
optimizer replay. The C compiler oracle is pinned Clang 22.1.8; the Rust SIMD
oracle is pinned Rust 1.90.0.

## Runtime protocols

Scalar regression uses `rotating-twelve-channel-v1`: candidate Native, current
Clang, replayed 0.11 Native/Clang, and replayed 0.10 Native/Clang, each in checked
and unchecked mode. All twelve streams execute in one process over identical
inputs. There are three rotating warm-up rows and twenty rotating sample rows,
seven calls per sample, the fixed batch identity, and upper-median statistics.
Schema 7 stores every actual order, sample, median, artifact digest, and result;
a missing stream cannot fall back to a historical number.

Vector and domain-fact suites use `rotating-three-channel-v1` separately for
checked and unchecked CK. A vector run rotates candidate CK, pinned hand-written
C+SIMD, and pinned hand-written Rust+SIMD. A domain-fact run substitutes generic
Clang O3 and Rust O3 sources that do not receive CK-only contracts. Each has
three warm-up rows, twenty sample rows, seven calls per sample, identical inputs,
fixed batching, and upper medians in one process.

Hand-written oracles use architecture-specific baseline flags, disable fast
math and contraction, and may not use a CPU feature absent from CK's baseline
profile. They receive every equivalent source-language precondition and must
pass differential and undefined-behavior auditing over the fixed declared valid
domain. Missing, invalid, or post-measurement-excluded competitors fail the gate.

## Cumulative release gates

- Existing Native/current-Clang and 0.10 replay gates remain: Native throughput
  is at least 95% of the Clang geometric mean, no item is more than 10% slower,
  checked proof loops reach at least 97% of unchecked throughput, and KIR
  optimizer latency is at most 2x in suite median and 3x individually versus
  the fixed 0.10 MIR optimizer.
- On each architecture and safety mode, CK reaches at least 95% of the geometric
  mean of the faster valid C/Rust SIMD oracle for each vector kernel, and every
  kernel reaches at least 90% of its oracle.
- On the domain-fact suite, CK exceeds the geometric mean of the faster generic
  Clang/Rust oracle by at least 5% on each architecture.
- The unchanged scalar corpus is no more than 3% slower in geometric mean and
  no individual case more than 8% slower than independently replayed 0.11.
- Native object size is no more than 35% larger in aggregate than replayed 0.11
  and no individual object exceeds 2.5x. Baseline O3 source-to-object compile
  time has a candidate/replay geometric mean at most 1.5 and individual ratios
  at most 2.

Runtime throughput, optimizer latency, source-to-object time, object size,
memory, cold/warm run, and cache behavior remain separate quantities. A threshold
never authorizes weaker diagnostics, evaluation order, modular integer behavior,
strict floating semantics, checked first-error order, print order, semantic MIR,
ABI, or contract domain.

## Running and retaining evidence

The schema/unit checks run locally before stable-worker measurement:

```sh
cargo bench --bench ckc_perf
cargo test --locked --test performance -- --nocapture
python3 -m unittest discover -s tests/performance -p '*_test.py'
python3 scripts/prepare-performance-replay.py --out target/ckc-perf/v011-replay
python3 scripts/prepare-performance-replay.py --baseline 0.10 --out target/ckc-perf/v010-replay
export CKC_V011_RUNTIME_BUNDLE="$PWD/target/ckc-perf/v011-replay"
export CKC_V010_RUNTIME_BUNDLE="$PWD/target/ckc-perf/v010-replay"
cargo build --release --features native-toolchain --locked
export CKC_CANDIDATE_COMPILER="$PWD/target/release/ckc"
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
python3 scripts/check-native-performance.py target/ckc-perf/results.json
```

The general compiler-stage benchmark writes `build/perf/latest.summary.json`
and `build/perf/latest.summary.md`; these summaries do not replace the strict
schema 7 release report.

Each preparation requires a separate fresh owned output directory, full Git history, Rust
1.90.0, and the pinned LLVM/Clang 22.1.8 installation. Replay bundles and reports
retain the compiler and oracle bytes, recipes/adapters, source manifests,
measurement directory, actual schedule/sample arrays, target-profile and
artifact digests, and preparation log. A missing or mutated file, symlink/path
escape, identity mismatch, quick run, unknown field, or incomplete sample is a
hard error.

The vector corpus includes contiguous map/zip, strict element-wise `f64`, integer
transforms, target-legal modular integer reductions, SLP, runtime no-alias
versioning, and specialization exposing a fixed slice length, with memory- and
compute-bound cases. Size uses paired relocatable objects before linking.
Compile-time uses fresh paths, disabled caches, three alternating warm-up pairs,
fifteen measured pairs, and upper medians.

Changing a source, compiler identity, threshold, statistic, target profile,
oracle precondition, or exclusion rule is a reviewed contract change.
PGO remains 0.13. Auto-Tuning remains 0.14.
