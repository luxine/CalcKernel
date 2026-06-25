# Performance

[简体中文](zh-CN/PERFORMANCE.md)

This document summarizes the Phase 14 local performance suite, current local
results, and how to run regression checks. Numbers are local measurements only:
do not compare absolute timings across machines. Results depend on hardware,
Node.js, clang, hyperfine, OS scheduling, power state, and current system load.

## Benchmark Suite

The hyperfine-based suite lives under `bench/perf` and targets
`examples/pricing.ik`, a helper-function fixture, and first-pass strict `f64`
compute kernels. It currently covers:

- native C unchecked: `pricing-c-unchecked-O0`, `pricing-c-unchecked-O2`,
  `pricing-c-unchecked-O3`, and `pricing-c-unchecked-ik-O3`
- native C checked: `pricing-c-checked-O3`
- LLVM unchecked: `pricing-llvm-unchecked-O0`, `pricing-llvm-unchecked-O2`,
  and `pricing-llvm-unchecked-O3`
- WASM unchecked total and compute-only cases, both at `IK-O0` and `IK-O3`
- WASM memory-only and JS-to-WASM call-overhead decomposition
- JavaScript baselines: `Number`, typed-array `Number`, and `BigInt`
- f64 kernels: axpy, dot product, sum, and scale
- f64 comparison targets: JavaScript `Array` `Number`, JavaScript
  `Float64Array`, IK C O3, IK LLVM O3, and IK WASM O3
- f64 WASM total, compute-only, and memory-only decomposition

The standard pricing workload is:

- 100,000 items
- 1,000 `calc_items` iterations
- checksum validation for every case

The standard f64 workload uses deterministic `Float64` inputs, consumes every
result checksum, and validates with absolute plus relative tolerance. It does
not require bit-identical floating point results across C, LLVM, WASM, and
JavaScript.

## Running Benchmarks

Benchmarks are manual release or development tools. They are intentionally not
part of ordinary `pnpm test`, and their thresholds must not be moved into the
unit test suite.

Quick local smoke:

```sh
node bench/perf/run.mjs --quick
```

Full local run:

```sh
node bench/perf/run.mjs --full
```

Run selected cases:

```sh
node bench/perf/run.mjs --quick --case pricing-c-unchecked
node bench/perf/run.mjs --full --case pricing-llvm-unchecked --case pricing-wasm-unchecked-compute-only
node bench/perf/run.mjs --quick --case f64
node bench/perf/run.mjs --quick --case f64-axpy
```

## Baselines and Regression Checks

Save a private local baseline:

```sh
node bench/perf/run.mjs --full --save-baseline
```

Compare against it:

```sh
node bench/perf/run.mjs --full --compare
node bench/perf/run.mjs --full --compare --threshold 10 --fail-on-regression
```

Regression checks use median runtime. The comparison report includes current
median, baseline median, runtime ratio, and slower percentage.

Baseline policy:

- Real local baselines are written to `build/perf/baseline.local.json`.
- `build/` is git-ignored; do not commit a developer-machine baseline.
- `bench/perf/baselines/example.summary.json` is only a format example.
- Ordinary `pnpm test` does not run hyperfine and does not fail due to
  performance variance.

Do not treat a local baseline as a cross-machine contract. Use `--compare` and
`--fail-on-regression` only for explicit local performance runs where the
machine and toolchain context are understood.

## Current Full Run Summary

Latest Phase 14 local full run on this machine, 2026-06-24:

| Case | Median ms | vs C O3 |
| --- | ---: | ---: |
| `pricing-c-unchecked-O0` | 620.272 | 10.75x |
| `pricing-c-unchecked-O2` | 56.983 | 0.99x |
| `pricing-c-unchecked-O3` | 57.696 | 1.00x |
| `pricing-c-unchecked-ik-O3` | 58.855 | 1.02x |
| `pricing-c-checked-O3` | 80.702 | 1.40x |
| `pricing-llvm-unchecked-O0` | 617.271 | 10.70x |
| `pricing-llvm-unchecked-O2` | 57.952 | 1.00x |
| `pricing-llvm-unchecked-O3` | 57.796 | 1.00x |
| `pricing-wasm-unchecked-compute-only` | 216.873 | 3.76x |
| `pricing-wasm-unchecked-compute-only-O3` | 115.737 | 2.01x |
| `pricing-wasm-unchecked-total` | 2721.645 | 47.17x |
| `pricing-wasm-unchecked-total-O3` | 2614.619 | 45.32x |
| `pricing-wasm-unchecked-memory-only` | 4765.740 | 82.60x |
| `pricing-js-typedarray-number` | 122.546 | 2.12x |
| `pricing-js-bigint` | 181.888 | 3.15x |

## Backend Comparison

Native unchecked C is the reference. Clang `-O2` and `-O3` produce roughly the
same result for the pricing kernel.

Checked C is about 40% slower than unchecked C in the current run. That overhead
comes from overflow checks, division checks, and status-return control flow. The
checked backend keeps business arithmetic checks such as `price * qty`; only a
proven-safe loop induction increment is optimized away at `-O3`.

LLVM `-O2` and `-O3` are effectively tied with native C for `pricing.ik`.
General LLVM functions still use alloca/load/store lowering, but clang promotes
the hot path well. Simple scalar straight-line functions can use a small
SSA-like lowering path at `-O2` and `-O3`.

WASM improved substantially in compute-only mode after simple while-loop
structured lowering and indexed address reuse. It remains slower than native C
and LLVM. The total WASM case is dominated by host-side `DataView` memory setup
and checksum reads, not by JS-to-WASM call overhead.

JavaScript `BigInt` remains useful as an exact `i64` baseline, but it is slower
than native C and LLVM. Typed-array `Number` is faster than `BigInt`, but it
does not provide exact `i64` semantics for all values.

The suite is not a NumPy-level or vectorized-library performance guarantee, and
it does not promise that WASM is always faster than JavaScript typed arrays.
WASM total time can be dominated by host memory marshaling.

C, LLVM-built native binaries, WASM, JavaScript, and any optional Python harness
do not share one runtime model. Use those comparisons to understand workload
shape, boundary cost, and safety tradeoffs; do not treat them as language
semantic tests or as absolute cross-runtime rankings.

For f64 kernels, JavaScript `Array` `Number`, JavaScript `Float64Array`, IK C,
IK LLVM, IK WASM, optional Python list `float`, and optional NumPy are different
runtime models. NumPy is a native-library baseline and is not a default runner
dependency. The f64 suite uses strict IK floating point only: no f32, fast-math,
SIMD, implicit int/float conversion, or f64 checked overflow is assumed.

## Batch Calling Principle

Always benchmark and ship batched calls:

```ik
export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32
```

Do not cross an FFI or JS-to-WASM boundary once per item. Boundary cost and host
memory marshaling can dominate if each item is called separately.

## WASM Notes

The current WASM backend is unchecked-only and does not provide a runtime,
allocator, or bounds checks. Host code owns memory layout, `DataView`
little-endian writes, and output buffer sizing.

For performance analysis, separate:

- total time: host memory setup + WASM compute + checksum read
- compute-only time: prewritten memory + repeated WASM calls
- memory-only time: host-side `DataView` work
- call-overhead time: JS-to-WASM boundary overhead

The f64 WASM benchmark follows the same separation for total, compute-only, and
memory-only time. f64 parameters and returns use JavaScript `Number`; i64/u64
pricing paths continue to use `BigInt` where required. f64 memory setup uses
little-endian `DataView.setFloat64` and `DataView.getFloat64`.

Current largest WASM bottleneck: host-side memory setup/readback for total
benchmarks. Compute-only WASM is much closer, but still slower than native code.

F64 benchmark interpretation is locked to strict semantics:

- quick runs are smoke checks, not release performance claims
- full runs are optional manual release checks
- do not put f64 performance thresholds into ordinary `pnpm test`
- do not commit machine-local f64 baselines
- compare finite f64 results with absolute and relative tolerance
- classify NaN, infinity, and `-0.0` instead of expecting bit-identical output
- treat JS `Array` `Number`, JS `Float64Array`, WASM, native C, LLVM, optional
  Python, and optional NumPy as different runtime models

## Checked vs Unchecked

Use unchecked mode when inputs are proven safe and maximum throughput matters.
Use checked C mode when arithmetic safety is required for money, tax, discount,
or rules calculations.

Checked mode currently applies to C output only. WASM and LLVM backends reject
`--overflow checked` rather than silently generating unchecked code.

## Current Biggest Bottlenecks

1. WASM total benchmark: host `DataView` memory setup and checksum readback.
2. WASM compute-only: generated WAT/VM execution is still about 2x native C O3.
3. Checked C: business overflow checks remain necessary and cost about 40%.
4. LLVM O0: stack lowering is intentionally slow without clang optimization.

Most valuable future work:

- broader WASM structured control-flow lowering
- reducing WASM i64 memory marshaling overhead in examples/benchmarks
- broader direct SSA LLVM lowering for non-memory scalar control flow
- optional, explicitly unsafe CPU-native/LTO experiments outside default builds
