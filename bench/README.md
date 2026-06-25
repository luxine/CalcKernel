# Pricing Benchmark Harness

[简体中文](README.zh-CN.md)

This directory contains small benchmark harnesses for `examples/pricing.ik`
and strict `f64` compute kernels. They compare pure JavaScript baselines,
generated native C, checked generated C where applicable, LLVM, and generated
WASM. They are intended as rough local references, not as a stable CI
performance suite. Results depend on the current machine, Node.js, clang,
hyperfine, and system load.

The benchmark sizes are:

- 100 items
- 1,000 items
- 10,000 items
- 100,000 items

## Local Hyperfine Performance Suite

For repeatable local performance testing, use the hyperfine-based runner:

```sh
brew install hyperfine
node bench/perf/run.mjs --quick
node bench/perf/run.mjs --full
```

`--quick` is useful while developing benchmark code. `--full` is the default
local performance run and uses more hyperfine samples and more work per command.
Both are manual commands; ordinary `pnpm test` does not run hyperfine and must
not gain machine-specific performance thresholds.

The runner performs the full local setup:

1. runs `pnpm build`
2. emits unchecked C, checked C, unchecked WASM, LLVM IR, and f64 artifacts
   into `build/perf/generated`
3. compiles C benchmark executables into `build/perf/bin`
4. smoke-runs each benchmark command for checksum validation
5. runs `hyperfine`
6. writes reports under `build/perf`

Output files:

- `build/perf/latest.hyperfine.json`
- `build/perf/latest.hyperfine.md`
- `build/perf/latest.summary.json`
- `build/perf/latest.summary.md`

To keep a private local baseline on this machine:

```sh
node bench/perf/run.mjs --full --save-baseline
node bench/perf/run.mjs --full --compare
```

By default, comparison reports regressions without failing the process. Use
`--fail-on-regression` when you want a non-zero exit code for local scripts:

```sh
node bench/perf/run.mjs --full --compare --fail-on-regression
```

The default regression threshold is 10% slower by median runtime. Override it
with `--threshold`:

```sh
node bench/perf/run.mjs --full --compare --threshold 5
node bench/perf/run.mjs --full --compare --threshold 10 --fail-on-regression
```

To run or compare a subset, repeat `--case`. Case filters match exact case names
or case-name prefixes:

```sh
node bench/perf/run.mjs --quick --case pricing-c-unchecked
node bench/perf/run.mjs --full --compare --case pricing-c-unchecked --case pricing-wasm-unchecked
```

The local baseline is stored in `build/perf/baseline.local.json`, which is not
intended to be committed. Do not compare absolute numbers across machines, and
do not commit a real baseline from a developer laptop. The checked-in
`bench/perf/baselines/example.summary.json` file is only a format example; it is
not a real threshold file.

Use `--compare`, `--threshold`, and `--fail-on-regression` only for explicit
local regression checks. They are not package correctness tests, and they should
not be treated as cross-machine guarantees.

The decomposed suite covers:

- `pricing-c-unchecked-O0`
- `pricing-c-unchecked-O2`
- `pricing-c-unchecked-O3`
- `pricing-c-unchecked-ik-O3`
- `pricing-c-checked-O3`
- `pricing-helpers-c-unchecked-ik-O0`
- `pricing-helpers-c-unchecked-ik-O2`
- `pricing-llvm-unchecked-O0`
- `pricing-llvm-unchecked-O2`
- `pricing-llvm-unchecked-O3`
- `pricing-wasm-unchecked-total`
- `pricing-wasm-unchecked-total-O3`
- `pricing-wasm-unchecked-compute-only`
- `pricing-wasm-unchecked-compute-only-O3`
- `pricing-wasm-unchecked-memory-only`
- `pricing-wasm-unchecked-call-overhead`
- `pricing-js-number`
- `pricing-js-typedarray-number`
- `pricing-js-bigint`

The first f64 suite covers four strict-float kernels:

- `axpy`: `y[i] = a * x[i] + y[i]`
- `dot`: `sum += x[i] * y[i]`
- `sum`: `sum += x[i]`
- `scale`: `x[i] = a * x[i]`

Each kernel has default comparison cases for:

- JavaScript `Array` with `Number` arithmetic
- JavaScript `Float64Array` with `Number` arithmetic
- IK C O3
- IK LLVM O3
- IK WASM O3 compute-only

The f64 WASM cases also include `total` and `memory-only` variants so host-side
memory marshaling can be separated from compute time. WASM f64 host interop
uses JavaScript `Number`; it does not use `BigInt`. Memory setup uses
little-endian `DataView.setFloat64`/`getFloat64`.

Run only f64 benchmarks with:

```sh
node bench/perf/run.mjs --quick --case f64
node bench/perf/run.mjs --full --case f64
```

Run one f64 kernel with:

```sh
node bench/perf/run.mjs --quick --case f64-axpy
node bench/perf/run.mjs --quick --case f64-dot
```

The summary includes each case's category, optimization level, arithmetic mode,
median runtime, p95 runtime, and ratio against `pricing-c-unchecked-O3`.

When `--compare` is enabled, `build/perf/latest.summary.md` also includes a
baseline comparison table. Regression status is based on median runtime:

- `ok`: at or below half the configured threshold
- `warning`: slower than half the threshold, but not over the threshold
- `regression`: slower than the configured threshold

The comparison table reports the current median, baseline median, runtime ratio,
and slower percentage. `--fail-on-regression` only affects explicit performance
runs; ordinary `pnpm test` does not run hyperfine and does not fail because of
machine performance variance.

The `pricing-helpers-*` cases use `bench/perf/fixtures/pricing_helpers.ik`,
which expresses the same pricing math through small non-exported helper
functions. It is a benchmark-only fixture for measuring MIR small-function
inlining; it does not change `examples/pricing.ik`.

The `f64-*` cases use `bench/perf/fixtures/f64_kernels.ik`. Correctness checks
use absolute and relative tolerance; they do not require cross-backend
bit-identical floating point results. IK f64 remains strict mode: no f32,
fast-math, SIMD, implicit int/float conversion, or f64 checked overflow is
assumed by these benchmarks.

Python list `float` and NumPy can be useful optional manual baselines, but they
are not default dependencies of this runner. NumPy is a native library baseline,
not a language semantics oracle.

F64 benchmark runs are documentation and release smoke tools:

- `--quick` is a smoke check
- `--full` is optional before tags when machine time allows
- f64 thresholds must not be added to ordinary `pnpm test`
- machine-local f64 baselines under `build/perf` must not be committed
- JS `Array` `Number`, JS `Float64Array`, IK C, IK LLVM, IK WASM, optional
  Python, and optional NumPy are different runtime models
- WASM total results may be dominated by host memory marshaling rather than
  compute

See [2026-06-24 Performance Profile](docs/2026-06-24-performance-profile.md)
for the current bottleneck analysis and Phase 14 optimization priorities.
See [Performance](../docs/PERFORMANCE.md) and
[Optimization](../docs/OPTIMIZATION.md) for the release-level summary of the
current pipeline, latest local full-run numbers, and regression workflow.

## Generate C

From the repository root:

```sh
pnpm build
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h --overflow unchecked
pnpm ikc emit-c examples/pricing.ik --out build/pricing.checked.c --header build/pricing.checked.h --overflow checked
```

## Generate WASM

From the repository root:

```sh
pnpm build
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm --overflow unchecked
```

The Phase 12 WASM backend is unchecked-only. `--overflow checked` is rejected
for `emit-wat` and `emit-wasm`; use the C backend when checked arithmetic is
required.

## Run the JavaScript Baseline

```sh
node bench/pricing_baseline.js
```

The JavaScript baseline uses `BigInt64Array` and `BigInt` arithmetic to stay
close to the `i64` semantics used by `pricing.ik`.

The local performance suite also includes three JavaScript pricing cases:

- `pricing-js-number`: plain JavaScript arrays and `Number` arithmetic.
- `pricing-js-typedarray-number`: `Float64Array` inputs and `Number`
  arithmetic.
- `pricing-js-bigint`: `BigInt64Array` inputs and `BigInt` arithmetic for
  exact `i64`-style calculations.

## Run the WASM Benchmark

Generate `build/pricing.wasm` first, then run:

```sh
node bench/wasm_pricing_benchmark.mjs
```

The WASM benchmark instantiates `build/pricing.wasm`, writes batched `Item`
arrays into exported linear memory with `DataView`, calls `calc_items`, and
reads the output buffer back from memory. It uses the same item sizes as the JS
and C harnesses:

- 100 items
- 1,000 items
- 10,000 items
- 100,000 items

The generated WASM module starts with one 64 KiB memory page. The benchmark
calls `memory.grow` on the host side when larger inputs need more space. This is
only benchmark setup code; IntKernel V0 still does not provide a runtime,
allocator, or memory-grow helper.

## Compile and Run the Unchecked C Benchmark

On macOS or Linux:

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL \
  build/pricing.c bench/pricing_c_harness.c \
  -I build \
  -o build/pricing_c_bench

./build/pricing_c_bench
```

On Windows with clang:

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL ^
  build\pricing.c bench\pricing_c_harness.c ^
  -I build ^
  -o build\pricing_c_bench.exe

build\pricing_c_bench.exe
```

## Compile and Run the Checked C Benchmark

On macOS or Linux:

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL \
  build/pricing.checked.c bench/pricing_checked_benchmark.c \
  -I build \
  -o build/pricing_checked_bench

./build/pricing_checked_bench
```

On Windows with clang:

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL ^
  build\pricing.checked.c bench\pricing_checked_benchmark.c ^
  -I build ^
  -o build\pricing_checked_bench.exe

build\pricing_checked_bench.exe
```

## Unchecked vs Checked

Unchecked mode emits direct C arithmetic and keeps the original C ABI:

```c
int32_t calc_items(Item* items, int32_t len, int64_t* out);
```

Checked mode emits additional branches and temporary values for overflow,
division-by-zero, and status propagation. Its ABI returns `IK_Status` and writes
the original IntKernel return value through a final output pointer:

```c
IK_Status calc_items(Item* items, int32_t len, int64_t* out, int32_t* ik_return);
```

The checked benchmark measures the same batch shape as the unchecked benchmark,
but it includes the cost of:

- `__builtin_add_overflow`, `__builtin_sub_overflow`, and
  `__builtin_mul_overflow`
- division and signed division overflow checks
- extra branches for `IK_Status` returns
- the final `ik_return` write

Checked mode is a better fit for money, tax, discount, and rules workloads where
integer safety is more important than maximum throughput. Unchecked mode is a
better fit for hot paths where inputs have already been proven to stay within
range.

## Reading Results

These numbers are only a rough reference. They can vary with CPU, compiler,
optimization flags, thermal state, operating system, and JavaScript engine
version.

For cross-language or WebAssembly calls, benchmark the shape you plan to ship.
Native FFI or JS-to-WASM call overhead can dominate if you call one item at a
time. Prefer batching many items per call, as `calc_items(items, len, out)` does
here.

C, LLVM-built native binaries, WASM, JavaScript, and optional Python harnesses
use different runtime and boundary models. Use benchmark comparisons as local
engineering signals, not as semantic tests or absolute cross-runtime rankings.

Do not compare per-item native calls against batched JavaScript loops; that
mostly measures FFI overhead. Compare batch calls of similar size.

WASM unchecked benchmark results are not checked-arithmetic safety results.
Unchecked WASM can be useful for portability and host integration experiments,
but it does not report integer overflow, division-by-zero safety, pointer
validity, or buffer length errors.

The decomposed WASM cases separate likely bottleneck layers:

- `pricing-wasm-unchecked-total`: writes memory, calls `calc_items`, and reads
  the checksum inside the measured workload.
- `pricing-wasm-unchecked-compute-only`: writes memory once, repeatedly calls
  `calc_items`, and reads the checksum once.
- `pricing-wasm-unchecked-memory-only`: measures host-side `DataView`
  memory write/read work without calling WASM.
- `pricing-wasm-unchecked-call-overhead`: repeatedly calls a tiny generated
  WASM function to estimate JS-to-WASM boundary cost.
