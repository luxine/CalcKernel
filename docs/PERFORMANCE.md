# Performance

[简体中文](zh-CN/PERFORMANCE.md)

This document summarizes the Phase 22 local performance suite, current local
results, and how to run regression checks. Numbers are local measurements only:
do not compare absolute timings across machines. Results depend on hardware,
Node.js, clang, hyperfine, OS scheduling, power state, and current system load.
For release-facing wording, see [v0.7.0 release notes](releases/v0.7.0.md).

## Benchmark Suite

The hyperfine-based suite lives under `bench/perf` and targets
`examples/pricing.ck`, a helper-function fixture, and first-pass strict `f64`
compute kernels. It currently covers:

- native C unchecked: `pricing-c-unchecked-O0`, `pricing-c-unchecked-O2`,
  `pricing-c-unchecked-O3`, and `pricing-c-unchecked-ck-O3`
- native C checked: `pricing-c-checked-O3`
- LLVM unchecked: `pricing-llvm-unchecked-O0`, `pricing-llvm-unchecked-O2`,
  and `pricing-llvm-unchecked-O3`
- WASM unchecked total and compute-only cases, both at `CK-O0` and `CK-O3`
- WASM memory-only and JS-to-WASM call-overhead decomposition
- WASM pricing SoA resident-memory cases using `BigInt64Array` views over
  exported memory
- JavaScript baselines: `Number`, typed-array `Number`, and `BigInt`
- f64 kernels: axpy, dot product, sum, and scale
- f64 comparison targets: JavaScript `Array` `Number`, JavaScript
  `Float64Array`, CK C O3, CK LLVM O3, and CK WASM O3
- f64 WASM setup, input marshal, compute-only, output readback, total, and
  memory-only decomposition
- f64 WASM low-copy variants using `Float64Array` views over exported memory

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
node bench/perf/run.mjs --full --case pricing-wasm-soa
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

Latest Phase 22 local full runs on this machine, 2026-06-26:

| Case | Median ms | Interpretation |
| --- | ---: | --- |
| `pricing-js-typedarray-number` | 123.519 | JS typed-array `Number` baseline |
| `pricing-js-bigint` | 181.427 | exact JS `BigInt` baseline |
| `pricing-wasm-unchecked-compute-only-O3` | 118.215 | WASM compute path, prewritten memory |
| `pricing-wasm-unchecked-total-O3` | 2562.414 | DataView AoS fallback total |
| `pricing-wasm-soa-setup-copy-in-O3` | 30.465 | one-time SoA resident copy-in |
| `pricing-wasm-soa-resident-total-O3` | 111.615 | recommended SoA resident path |
| `pricing-wasm-soa-readback-cost-O3` | 261.313 | repeated output readback cost |
| `pricing-wasm-soa-total-with-final-readback-O3` | 118.219 | resident compute plus one final readback |

| Case | Median ms | Interpretation |
| --- | ---: | --- |
| `f64-sum-js-float64array` | 112.375 | JS `Float64Array` baseline |
| `f64-sum-ck-wasm-o3-compute-only` | 90.739 | WASM compute path |
| `f64-sum-ck-wasm-o3-optimized-low-copy-total` | 89.150 | recommended resident/scalar-return path |
| `f64-sum-ck-wasm-o3-total` | 1033.140 | DataView fallback total |
| `f64-axpy-js-float64array` | 114.551 | JS `Float64Array` baseline |
| `f64-axpy-ck-wasm-o3-compute-only` | 99.539 | WASM compute path |
| `f64-axpy-ck-wasm-o3-view-output-total` | 114.269 | recommended output-view path |
| `f64-axpy-ck-wasm-o3-copy-output-total` | 204.850 | explicit copy-output path |
| `f64-axpy-ck-wasm-o3-total` | 1147.689 | DataView fallback total |

These tables do not mean "CK WASM is faster than JS" in general. They show that
the CK WASM compute path is competitive, and that resident memory, SoA layout,
typed-array bulk copy, scalar returns, and output views can make selected
batched workloads faster than the corresponding JavaScript typed-array
baseline. Mixed-width struct marshaling with `DataView` and large output
copy/readback can make WASM total time much slower.

## Backend Comparison

Native unchecked C is the reference. Clang `-O2` and `-O3` produce roughly the
same result for the pricing kernel.

Checked C is about 40% slower than unchecked C in the current run. That overhead
comes from overflow checks, division checks, and status-return control flow. The
checked backend keeps business arithmetic checks such as `price * qty`; only a
proven-safe loop induction increment is optimized away at `-O3`.

LLVM `-O2` and `-O3` are effectively tied with native C for `pricing.ck`.
General LLVM functions still use alloca/load/store lowering, but clang promotes
the hot path well. Simple scalar straight-line functions can use a small
SSA-like lowering path at `-O2` and `-O3`.

WASM improved substantially in compute-only mode after simple while-loop
structured lowering and indexed address reuse. It remains slower than native C
and LLVM. The total WASM case is dominated by host-side `DataView` memory setup
and checksum reads, not by JS-to-WASM call overhead.

For pricing, Phase 22 adds a recommended SoA resident-memory benchmark fixture:

```ck
export fn pricing_soa(
  prices: ptr<i64>,
  quantities: ptr<i64>,
  discounts: ptr<i64>,
  tax_rates_ppm: ptr<i64>,
  out_totals: ptr<i64>,
  n: i32
) -> i32
```

The fixture keeps the same integer fixed-point arithmetic as `calc_items`, but
lays input out as homogeneous arrays. JavaScript uses `BigInt64Array#set` to
bulk-copy inputs once into WASM memory, keeps output in WASM memory, and reports
readback cost separately. This is the recommended pricing interop shape for
large resident batches. The original mixed-width/AoS `DataView` path remains a
fallback/debug ABI comparison.

JavaScript `BigInt` remains useful as an exact `i64` baseline, but it is slower
than native C and LLVM. Typed-array `Number` is faster than `BigInt`, but it
does not provide exact `i64` semantics for all values.

The suite is not a NumPy-level or vectorized-library performance guarantee, and
it does not turn WASM into a blanket replacement for JavaScript typed-array hot
loops. WASM total time can be dominated by host memory marshaling.

C, LLVM-built native binaries, WASM, JavaScript, and any optional Python harness
do not share one runtime model. Use those comparisons to understand workload
shape, boundary cost, and safety tradeoffs; do not treat them as language
semantic tests or as absolute cross-runtime rankings.

For f64 kernels, JavaScript `Array` `Number`, JavaScript `Float64Array`, CK C,
CK LLVM, CK WASM, optional Python list `float`, and optional NumPy are different
runtime models. NumPy is a native-library baseline and is not a default runner
dependency. The f64 suite uses strict CK floating point only: `f64` is the only
floating point type, `f32` is not planned, and no fast-math, SIMD, implicit
int/float conversion, broad casts, or f64 checked overflow is assumed. The only
current numeric casts are exact explicit `i32_to_f64` and `u32_to_f64` builtins.
JavaScript `Float64Array` can be a strong baseline for tight host loops. WASM
compute-only and WASM total answer different questions, so do not conclude that
WASM is faster or slower than JavaScript without checking which phase is being
measured.

## Batch Calling Principle

Always benchmark and ship batched calls:

```ck
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

The f64 WASM benchmark exposes a more detailed Phase 18.1 split:

- setup: instantiate the f64 WASM module and provision linear memory
- input-marshal: write deterministic `f64` inputs into WASM memory
- compute-only: prewritten memory + repeated WASM kernel calls
- output-readback: read computed `f64` buffers back from WASM memory
- total: input marshal + WASM compute + output readback in one measured path
- memory-only: host-side write + readback without executing the WASM kernel

The generated summary includes a `Phase` column so these paths are visible in
`build/perf/latest.summary.md`. f64 parameters and returns use JavaScript
`Number`; i64/u64 pricing paths continue to use `BigInt` where required.
Phase 18.2 adds `examples/node-wasm-f64-array/` as the recommended host pattern
for bulk `ptr<f64>` buffers: create a `Float64Array` view over exported WASM
memory, convert byte offsets with `byteOffset / 8`, and use typed-array bulk
operations instead of per-element `DataView` calls in the hot path. `DataView`
remains the byte-level ABI tool for mixed-width struct checks.

Phase 18.3 adds low-copy f64 WASM benchmark cases named
`f64-*-ck-wasm-o3-low-copy-*`. These cases keep the same WASM pointer ABI but
measure the recommended host path separately: `Float64Array#set` for input
marshal, WASM compute, scalar return consume for `dot`/`sum`, and output checksum
readback for in-place array kernels. The original DataView cases remain in the
suite so byte-level marshaling overhead stays visible.

Phase 22 adds CKWasmArena-backed f64 optimized benchmark cases for the two
primary JS/WASM interop shapes:

- `f64-sum-ck-wasm-o3-optimized-low-copy-total` copies `Float64Array` input into
  WASM memory once, keeps it resident, repeatedly calls the strict `sum_f64`
  kernel, and consumes the scalar `f64` return without output readback. This is
  the recommended CK WASM shape when the workload is a reduction over resident
  homogeneous `f64` data.
- `f64-axpy-ck-wasm-o3-view-output-total` copies resident `x` input once,
  refreshes `y`/output with `Float64Array#set`, lets the WASM kernel write
  output into WASM memory, and keeps output as a WASM memory view. This is the
  recommended in-place/output buffer shape.
- `f64-axpy-ck-wasm-o3-copy-output-total` explicitly copies output to a
  JS-owned `Float64Array`. Use this when ownership requires it, but expect copy
  out to weaken or remove the WASM advantage.

Use the CKWasmArena low-copy/view-output path for production-style homogeneous
f64 buffers. Use DataView when byte offsets, mixed-width structs, and ABI
precision matter more than hot-path throughput. The DataView total cases remain
fallback comparisons, not the recommended path for large f64 buffers.

Phase 22 also adds CKWasmArena-backed pricing SoA cases:

- `pricing-wasm-soa-setup-copy-in-O3` measures one-time arena allocation,
  memory growth, and `BigInt64Array#set` copy-in for resident pricing arrays.
- `pricing-wasm-soa-resident-total-O3` copies input once, repeatedly calls
  `pricing_soa`, checks the scalar `i32` status return, and leaves output as a
  WASM memory view.
- `pricing-wasm-soa-readback-cost-O3` isolates repeated `BigInt64Array` output
  view checksum/readback cost.
- `pricing-wasm-soa-total-with-final-readback-O3` measures resident compute plus
  one final output view checksum.

Prefer SoA plus resident memory for pricing workloads when JavaScript can keep
data in homogeneous typed arrays. Do not use the `DataView` pricing total as the
recommended performance path; it exists to keep mixed-width struct ABI cost
visible.

Official runnable examples for the recommended interop shapes live under
`examples/wasm`:

- [`examples/wasm/f64-sum`](../examples/wasm/f64-sum/README.md): read-only
  `Float64Array` input with a scalar `f64` return and no output readback.
- [`examples/wasm/f64-axpy`](../examples/wasm/f64-axpy/README.md): output view
  fast path, with `copyOutF64` shown only as an explicit JS-owned copy.
- [`examples/wasm/pricing-soa`](../examples/wasm/pricing-soa/README.md): SoA
  integer fixed-point pricing using `BigInt64Array` views over WASM memory.

Run them after building the package:

```sh
pnpm build
node examples/wasm/f64-sum/run.mjs
node examples/wasm/f64-axpy/run.mjs
node examples/wasm/pricing-soa/run.mjs
```

`Float64Array` views must be recreated after `memory.grow`; CK does not provide
a WASM allocator or runtime, so host code still owns memory placement and buffer
sizing.

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
- interpret DataView total and low-copy total separately; neither is a
  cross-machine performance guarantee

## Checked vs Unchecked

Use unchecked mode when inputs are proven safe and maximum throughput matters.
Use checked C mode when arithmetic safety is required for money, tax, discount,
or rules calculations.

Checked mode currently applies to C output only. WASM and LLVM backends reject
`--overflow checked` rather than silently generating unchecked code.

## Current Biggest Bottlenecks

1. WASM DataView total benchmark: host `DataView` memory setup and checksum
   readback.
2. Pricing and f64 copy-out/readback: large output reads can erase compute-only
   WASM gains.
3. WASM compute-only: generated WAT/VM execution is still slower than native C
   O3.
4. Checked C: business overflow checks remain necessary and cost about 40%.
5. LLVM O0: stack lowering is intentionally slow without clang optimization.

Most valuable future work:

- broader WASM structured control-flow lowering
- preferring pricing SoA resident-memory interop over mixed-width AoS DataView
  hot paths
- broader direct SSA LLVM lowering for non-memory scalar control flow
- optional, explicitly unsafe CPU-native/LTO experiments outside default builds
