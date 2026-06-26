# Phase 22 WASM-JS Interop Performance Plan

Date: 2026-06-26

Scope: CK / CalcKernel Phase 22 focuses on WASM-JS interop overhead in the
local performance benchmark layer. The goal is to make benchmark results
describe what is being measured, then optimize host memory traffic without
changing CK language semantics, `ckc`, `.ck` syntax, the `calckernel` package
surface, or the `CK_` C ABI.

This phase does not add SIMD, `f32`, fast-math, runtime allocation, GC, IO,
strings, benchmark pass/fail thresholds in `pnpm test`, published baselines,
tags, commits, or npm publishing.

## Current Benchmark Structure

The benchmark runner is `bench/perf/run.mjs`.

- Generates C, WASM, and LLVM artifacts under `build/perf/generated`.
- Compiles native harnesses under `build/perf/bin`.
- Runs `hyperfine`.
- Writes ignored local outputs under `build/perf/latest.*`.
- Summarizes results through `bench/perf/lib/summary.mjs`.

Benchmark case definitions live in `bench/perf/lib/cases.mjs`.

Current pricing cases:

- Native C and LLVM totals.
- JS `Number` total.
- JS TypedArray total.
- JS BigInt total.
- WASM DataView total.
- WASM compute-only.
- WASM memory-only.
- WASM scalar call-overhead.
- WASM SoA setup/copy-in.
- WASM SoA resident total.
- WASM SoA readback cost.
- WASM SoA total with one final readback.

Current f64 cases for `axpy`, `dot`, `sum`, and `scale`:

- JS `Array` `Number` total.
- JS `Float64Array` total.
- CK C O3 total.
- CK LLVM O3 total.
- WASM DataView setup.
- WASM DataView input marshal.
- WASM DataView compute-only.
- WASM DataView output readback.
- WASM DataView total.
- WASM DataView memory-only.
- WASM low-copy setup.
- WASM low-copy input marshal.
- WASM low-copy compute-only.
- WASM low-copy output readback.
- WASM low-copy total.

Task 22.1 adds summary metadata for:

- `benchmarkLayer`
- `dataViewHotPath`
- `copyInput`
- `copyOutput`
- `outputOwnership`
- `memoryGrow`

These fields are descriptive benchmark metadata. They do not change the
measured code paths.

## Located Bottlenecks

Pricing total is dominated by JS-side memory marshaling, not by the CK / CalcKernel
WASM compute kernel. The current WASM total path writes four `i64` fields per
item with `DataView.setBigInt64`, creates BigInt values in the hot loop, calls
the kernel, then reads every output element with `DataView.getBigInt64`.

Task 22.4 adds a pricing SoA fixture:

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

It keeps the same fixed-point integer semantics as `calc_items`, but avoids the
mixed-width/AoS DataView hot path by copying homogeneous `BigInt64Array` inputs
once into resident WASM memory.

f64 DataView total is dominated by per-element `DataView.setFloat64` input
marshaling and per-element `DataView.getFloat64` checksum/readback. This is a
byte-exact ABI fallback path, not the preferred homogeneous `f64` bulk path.

f64 low-copy total removes DataView from the hot path by using `Float64Array`
views over WASM memory. It is effective for scalar-return kernels such as
`sum`, but in-place output kernels such as `axpy` and `scale` still pay a
per-iteration output checksum over the WASM memory view.

Memory growth is host-managed. The benchmark may call `memory.grow` during
setup when the exported WASM memory is not large enough for the configured
item count. Typed views must be recreated after growth.

## DataView Hot Path List

| Location | Classification | Notes |
| --- | --- | --- |
| `bench/perf/cases/pricing-wasm.mjs:72` | hot path not acceptable | `runTotal` repeats DataView input marshal, kernel call, and output checksum. |
| `bench/perf/cases/pricing-wasm.mjs:112` | debug/fallback acceptable | `runMemoryOnly` isolates host DataView memory cost; keep as a diagnostic fallback. |
| `bench/perf/cases/pricing-wasm.mjs:187` | scalar/header metadata access | `ensureMemory` may grow memory and recreates a `DataView`; acceptable setup, must be labeled. |
| `bench/perf/cases/pricing-wasm.mjs:199` | mixed-width struct marshal | `writeItems` writes the pricing AoS `Item` layout through four `setBigInt64` calls per item. |
| `bench/perf/cases/pricing-wasm.mjs:210` | debug/fallback acceptable | `writeExpectedOut` is memory-only fallback setup. |
| `bench/perf/cases/pricing-wasm.mjs:240` | output readback | `checksum` reads every output element through `getBigInt64`. |
| `bench/perf/cases/f64-wasm.mjs:154` | hot path not acceptable | DataView `input-marshal` repeats per-element input writes. |
| `bench/perf/cases/f64-wasm.mjs:210` | hot path not acceptable | DataView total repeats input marshal and output checksum every iteration. |
| `bench/perf/cases/f64-wasm.mjs:244` | output readback | DataView output-readback isolates per-element memory reads. |
| `bench/perf/cases/f64-wasm.mjs:275` | debug/fallback acceptable | DataView memory-only isolates host memory traffic without a WASM module. |
| `bench/perf/cases/f64-wasm.mjs:330` | hot path not acceptable | `writeInputs` uses `setFloat64` per element. |
| `bench/perf/cases/f64-wasm.mjs:345` | output readback | `checksumMemory` uses `getFloat64` per element. |
| `bench/perf/lib/f64-wasm-memory.mjs:23` | scalar/header metadata access | `ensureDataView` is a setup helper for the DataView fallback path. |

DataView use in WASM e2e tests, backend regression tests, ABI docs, and Node
or browser examples is acceptable when it demonstrates byte-exact ABI behavior
or mixed-width struct packing. It should not be treated as the optimized hot
path for large homogeneous buffers.

## Current Low-Copy Paths

| Location | Layer | Notes |
| --- | --- | --- |
| `bench/perf/cases/f64-wasm.mjs:167` | WASM setup/copy-in | Uses `Float64Array#set` for repeated input marshal. |
| `bench/perf/cases/f64-wasm.mjs:195` | WASM compute-only | Writes input once with `Float64Array#set`, then repeats kernel calls. |
| `bench/perf/cases/f64-wasm.mjs:227` | WASM low-copy total | Uses `Float64Array#set`, kernel call, and output checksum per iteration. |
| `bench/perf/cases/f64-wasm.mjs:259` | WASM readback/copy-out | Uses a `Float64Array` view checksum instead of DataView reads. |
| `bench/perf/lib/f64-wasm-memory.mjs:28` | helper | Recreates `Float64Array` views after possible `memory.grow`. |
| `bench/perf/lib/f64-wasm-memory.mjs:41` | helper | Bulk input copy through `Float64Array#set`. |
| `bench/perf/lib/f64-wasm-memory.mjs:49` | helper | Returns scalar results directly for `dot` and `sum`; checks WASM memory view for `axpy` and `scale`. |
| `bench/perf/cases/pricing-wasm.mjs` | WASM SoA resident | Uses CKWasmArena `BigInt64Array` views and `set` bulk-copy for pricing inputs; output remains a WASM memory view. |
| `bench/perf/fixtures/pricing_soa.ck` | benchmark fixture | Provides the SoA `ptr<i64>` pricing kernel for interop benchmarking without changing language semantics. |

## Optimization Direction

1. Introduce a benchmark-only `CKWasmMemory` or `CKWasmArena` helper for memory
   sizing, aligned offsets, view recreation after growth, and typed-array view
   lookup.
2. Package the existing f64 `Float64Array` low-copy path behind that helper so
   benchmark cases and examples use one memory-management pattern.
3. Add resident-memory benchmarks: copy input once, call the kernel many times,
   and separate final correctness readback from the timed kernel loop.
4. Add pricing SoA benchmarks to compare the current mixed-width AoS/DataView
   path with bulk typed-array movement. This may require a separate benchmark
   fixture, but it should not require a WASM backend ABI change.
5. Add a f64 `axpy` view-output fast path benchmark that reports both
   per-iteration output checksum cost and resident output-in-WASM cost.
6. Keep DataView fallback cases in the suite for ABI debugging and regression
   visibility, but label them as fallback or memory-only in summaries.

## ABI Assessment

Task 22.1 does not require a WASM backend ABI change.

The next optimization tasks can be completed through JS helpers, examples, and
benchmark fixtures. A pricing SoA benchmark may add new `.ck` benchmark fixture
functions, but it should not alter parser, type checker, MIR, C backend, LLVM
backend, f64 strict semantics, or the existing WASM pointer ABI.

## Task 22.5 Claims Policy

Do not use these claims:

- CK WASM is faster than JS.
- WASM is always faster than JS `TypedArray`.
- Compiling CK / CalcKernel to WASM automatically makes code faster.
- The mixed-width/AoS `DataView` pricing path is a high-performance path.

Use these narrower claims instead:

- CK / CalcKernel WASM compute paths are competitive in the measured workloads.
- Batched `f64` read-only/scalar-return workloads can benefit from low-copy
  WASM memory and may beat JavaScript `Float64Array` on a given machine.
- Resident memory can reduce JS-WASM boundary cost by avoiding repeated input
  marshaling.
- SoA plus typed-array bulk copy is the recommended WASM interop path for
  pricing-style batch workloads when the host can shape data that way.
- Mixed-width struct marshaling, per-field `DataView`, and large output
  readback can make WASM total time much slower than JavaScript typed-array
  baselines.
- Copy-output is an explicit slow path; output view is the recommended fast
  path when JS-owned storage is not required.
- Benchmark results depend on machine, Node/V8 version, compiler/toolchain,
  workload size, and system load.

Latest local Phase 22 full-run highlights, 2026-06-26:

| Case | Median ms | Claim boundary |
| --- | ---: | --- |
| `f64-sum-js-float64array` | 112.375 | JS `Float64Array` baseline |
| `f64-sum-ck-wasm-o3-optimized-low-copy-total` | 89.150 | recommended scalar-return low-copy path |
| `f64-sum-ck-wasm-o3-total` | 1033.140 | DataView fallback total |
| `f64-axpy-js-float64array` | 114.551 | JS `Float64Array` baseline |
| `f64-axpy-ck-wasm-o3-view-output-total` | 114.269 | recommended output-view path |
| `f64-axpy-ck-wasm-o3-copy-output-total` | 204.850 | explicit copy-output path |
| `pricing-js-typedarray-number` | 123.519 | JS typed-array `Number` baseline |
| `pricing-wasm-soa-resident-total-O3` | 111.615 | recommended SoA resident path |
| `pricing-wasm-unchecked-total-O3` | 2562.414 | DataView AoS fallback total |

## Acceptance Criteria For Phase 22 Follow-Up

- Summary JSON and markdown identify DataView hot path participation.
- Summary JSON and markdown identify input copy, output copy, output ownership,
  and memory growth behavior.
- DataView fallback paths remain runnable and clearly labeled.
- Low-copy paths remain correctness-checked.
- Resident-memory and pricing SoA benchmarks produce local `build/perf` output
  without committing machine-specific baselines.
