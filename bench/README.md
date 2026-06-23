# Pricing Benchmark Harness

This directory contains small benchmark harnesses for `examples/pricing.ik`.
They are intended as rough local references, not as a stable CI performance
suite.

The benchmark sizes are:

- 100 items
- 1,000 items
- 10,000 items
- 100,000 items

## Generate C

From the repository root:

```sh
pnpm build
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h --overflow unchecked
pnpm ikc emit-c examples/pricing.ik --out build/pricing.checked.c --header build/pricing.checked.h --overflow checked
```

## Run the JavaScript Baseline

```sh
node bench/pricing_baseline.js
```

The JavaScript baseline uses `BigInt64Array` and `BigInt` arithmetic to stay
close to the `i64` semantics used by `pricing.ik`.

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

For cross-language calls, benchmark the shape you plan to ship. Native FFI call
overhead can dominate if you call one item at a time. Prefer batching many items
per native call, as `calc_items(items, len, out)` does here.

Do not compare per-item native calls against batched JavaScript loops; that
mostly measures FFI overhead. Compare batch calls of similar size.
