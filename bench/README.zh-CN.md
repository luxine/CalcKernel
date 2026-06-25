# Pricing Benchmark Harness

[English](README.md)

本目录包含针对 `examples/pricing.ik` 和 strict `f64` compute kernel 的小型
benchmark harness，用于对比纯 JavaScript baseline、生成的 native C、可适用时
的 checked C、LLVM，以及生成的 WASM。它们只是粗略的本地参考，不是稳定的 CI
性能套件。结果依赖当前机器、Node.js、clang、hyperfine 和系统负载。

Benchmark 规模：

- 100 items
- 1,000 items
- 10,000 items
- 100,000 items

## 本机 Hyperfine 性能套件

需要更稳定的本机 performance test 时，使用基于 hyperfine 的 runner：

```sh
brew install hyperfine
node bench/perf/run.mjs --quick
node bench/perf/run.mjs --full
```

`--quick` 适合开发 benchmark 代码时快速检查。`--full` 是默认的本机性能测试，
会使用更多 hyperfine samples 和更多内部循环。两者都是手动命令；普通 `pnpm test`
不运行 hyperfine，也不能加入机器相关性能阈值。

runner 会完成完整本机准备工作：

1. 运行 `pnpm build`
2. 生成 unchecked C、checked C、unchecked WASM、LLVM IR 和 f64 artifacts 到
   `build/perf/generated`
3. 编译 C benchmark 可执行文件到 `build/perf/bin`
4. 先 smoke-run 每个 benchmark 命令并校验 checksum
5. 运行 `hyperfine`
6. 写出 `build/perf` 下的报告

输出文件：

- `build/perf/latest.hyperfine.json`
- `build/perf/latest.hyperfine.md`
- `build/perf/latest.summary.json`
- `build/perf/latest.summary.md`

在本机保存私有 baseline：

```sh
node bench/perf/run.mjs --full --save-baseline
node bench/perf/run.mjs --full --compare
```

默认情况下，compare 只报告 regression，不会让进程失败。如果要给本机脚本使用
非零退出码，加上 `--fail-on-regression`：

```sh
node bench/perf/run.mjs --full --compare --fail-on-regression
```

默认 regression 阈值是 median runtime 变慢 10%。可以用 `--threshold` 覆盖：

```sh
node bench/perf/run.mjs --full --compare --threshold 5
node bench/perf/run.mjs --full --compare --threshold 10 --fail-on-regression
```

如果只想运行或比较部分 case，可以重复传入 `--case`。case filter 支持精确 case
名称，也支持 case-name prefix：

```sh
node bench/perf/run.mjs --quick --case pricing-c-unchecked
node bench/perf/run.mjs --full --compare --case pricing-c-unchecked --case pricing-wasm-unchecked
```

本机 baseline 保存在 `build/perf/baseline.local.json`，不应提交。不要跨机器比较
绝对性能数字，也不要提交开发机器上的真实 baseline。仓库中的
`bench/perf/baselines/example.summary.json` 只是格式示例，不是真实阈值文件。

`--compare`、`--threshold` 和 `--fail-on-regression` 只用于显式本机 regression
检查。它们不是 package correctness tests，也不应被当作跨机器保证。

拆解后的套件包含：

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

第一版 f64 套件覆盖四个 strict-float kernel：

- `axpy`：`y[i] = a * x[i] + y[i]`
- `dot`：`sum += x[i] * y[i]`
- `sum`：`sum += x[i]`
- `scale`：`x[i] = a * x[i]`

每个 kernel 默认包含以下对比 case：

- JavaScript `Array` + `Number` arithmetic
- JavaScript `Float64Array` + `Number` arithmetic
- IK C O3
- IK LLVM O3
- IK WASM O3 compute-only

f64 WASM case 还包含 `total` 和 `memory-only` 变体，用于把 host-side memory
marshal time 和 compute time 拆开观察。WASM f64 host interop 使用 JavaScript
`Number`，不使用 `BigInt`。memory setup 使用 little-endian
`DataView.setFloat64`/`getFloat64`。

只运行 f64 benchmark：

```sh
node bench/perf/run.mjs --quick --case f64
node bench/perf/run.mjs --full --case f64
```

只运行单个 f64 kernel：

```sh
node bench/perf/run.mjs --quick --case f64-axpy
node bench/perf/run.mjs --quick --case f64-dot
```

summary 会包含每个 case 的 category、optimization level、arithmetic mode、
median runtime、p95 runtime，以及相对于 `pricing-c-unchecked-O3` 的倍数。

启用 `--compare` 后，`build/perf/latest.summary.md` 还会包含 baseline comparison
表。regression status 基于 median runtime：

- `ok`：未超过配置阈值的一半
- `warning`：超过阈值一半，但未超过完整阈值
- `regression`：超过配置阈值

comparison 表会输出当前 median、baseline median、runtime ratio 和变慢百分比。
`--fail-on-regression` 只影响显式的性能运行；普通 `pnpm test` 不运行 hyperfine，
也不会因为机器性能波动失败。

`pricing-helpers-*` case 使用 `bench/perf/fixtures/pricing_helpers.ik`，
它把相同 pricing 计算拆成小型 non-exported helper function。这个 fixture
只用于测 MIR small-function inlining，不会改变 `examples/pricing.ik`。

`f64-*` case 使用 `bench/perf/fixtures/f64_kernels.ik`。正确性检查使用 absolute
tolerance 和 relative tolerance；不要求跨 backend 浮点结果 bit-identical。
IK f64 仍是 strict mode：这些 benchmark 不假设 f32、fast-math、SIMD、隐式
int/float conversion 或 f64 checked overflow。

Python list `float` 和 NumPy 可以作为可选手动 baseline，但不是本 runner 的默认
依赖。NumPy 是 native library baseline，不是语言语义 oracle。

f64 benchmark run 是文档和 release smoke 工具：

- `--quick` 是 smoke check
- `--full` 是 tag 前可选手动检查
- 不把 f64 阈值加入普通 `pnpm test`
- 不提交 `build/perf` 下的本机 f64 baseline
- JS `Array` `Number`、JS `Float64Array`、IK C、IK LLVM、IK WASM、可选 Python
  和可选 NumPy 是不同 runtime model
- WASM total 结果可能主要受 host memory marshal 影响，而不是 compute 本身

当前瓶颈分析和 Phase 14 优化优先级见
[2026-06-24 性能画像](docs/2026-06-24-performance-profile.zh-CN.md)。
当前 pipeline、本机最新 full-run 数字和回归流程的 release-level 总结见
[Performance](../docs/zh-CN/PERFORMANCE.md) 和
[Optimization](../docs/zh-CN/OPTIMIZATION.md)。

## 生成 C

从仓库根目录执行：

```sh
pnpm build
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h --overflow unchecked
pnpm ikc emit-c examples/pricing.ik --out build/pricing.checked.c --header build/pricing.checked.h --overflow checked
```

## 生成 WASM

从仓库根目录执行：

```sh
pnpm build
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm --overflow unchecked
```

Phase 12 WASM backend 只支持 unchecked。`emit-wat` 和 `emit-wasm` 会拒绝
`--overflow checked`；需要 checked arithmetic 时请使用 C backend。

## 运行 JavaScript Baseline

```sh
node bench/pricing_baseline.js
```

JavaScript baseline 使用 `BigInt64Array` 和 `BigInt` arithmetic，尽量贴近
`pricing.ik` 使用的 `i64` 语义。

本机 performance suite 还包含三个 JavaScript pricing case：

- `pricing-js-number`：普通 JavaScript array 和 `Number` arithmetic。
- `pricing-js-typedarray-number`：`Float64Array` input 和 `Number`
  arithmetic。
- `pricing-js-bigint`：`BigInt64Array` input 和 `BigInt` arithmetic，用于精确
  模拟 `i64` 风格计算。

## 运行 WASM Benchmark

先生成 `build/pricing.wasm`，然后运行：

```sh
node bench/wasm_pricing_benchmark.mjs
```

WASM benchmark 会实例化 `build/pricing.wasm`，用 `DataView` 将批量 `Item`
array 写入导出的 linear memory，调用 `calc_items`，再从 memory 读取 output
buffer。它使用和 JS/C harness 相同的数据规模：

- 100 items
- 1,000 items
- 10,000 items
- 100,000 items

生成的 WASM module 初始只有一个 64 KiB memory page。较大输入需要更多空间时，
benchmark 会在 host 侧调用 `memory.grow`。这只是 benchmark setup code；
IntKernel V0 仍不提供 runtime、allocator 或 memory-grow helper。

## 编译并运行 Unchecked C Benchmark

macOS 或 Linux：

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL \
  build/pricing.c bench/pricing_c_harness.c \
  -I build \
  -o build/pricing_c_bench

./build/pricing_c_bench
```

Windows with clang：

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL ^
  build\pricing.c bench\pricing_c_harness.c ^
  -I build ^
  -o build\pricing_c_bench.exe

build\pricing_c_bench.exe
```

## 编译并运行 Checked C Benchmark

macOS 或 Linux：

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL \
  build/pricing.checked.c bench/pricing_checked_benchmark.c \
  -I build \
  -o build/pricing_checked_bench

./build/pricing_checked_bench
```

Windows with clang：

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL ^
  build\pricing.checked.c bench\pricing_checked_benchmark.c ^
  -I build ^
  -o build\pricing_checked_bench.exe

build\pricing_checked_bench.exe
```

## Unchecked vs Checked

Unchecked mode 生成直接 C arithmetic，并保持原始 C ABI：

```c
int32_t calc_items(Item* items, int32_t len, int64_t* out);
```

Checked mode 为 overflow、division-by-zero 和 status propagation 生成额外分支和
临时值。它的 ABI 返回 `IK_Status`，并通过最后一个 output pointer 写出原始
IntKernel return value：

```c
IK_Status calc_items(Item* items, int32_t len, int64_t* out, int32_t* ik_return);
```

Checked benchmark 测量和 unchecked benchmark 相同的 batch shape，但包含以下成本：

- `__builtin_add_overflow`、`__builtin_sub_overflow` 和 `__builtin_mul_overflow`
- division 和 signed division overflow checks
- `IK_Status` return 的额外分支
- 最终 `ik_return` 写入

Checked mode 更适合金额、税费、优惠和规则 workload，这些场景中整数安全比最大吞吐
更重要。Unchecked mode 更适合已经证明输入不会越界的热路径。

## 解读结果

这些数字只是粗略参考。它们会随 CPU、compiler、optimization flags、温度状态、
操作系统和 JavaScript engine version 变化。

跨语言或 WebAssembly 调用时，应 benchmark 你计划实际发布的调用形状。如果一条
item 调一次 native/wasm 函数，FFI 或 JS-to-WASM call overhead 可能占主导。优先像
`calc_items(items, len, out)` 这样每次调用批量处理多个 item。

C、LLVM 构建出的 native binary、WASM、JavaScript，以及可选 Python harness 使用
不同 runtime 和 boundary model。Benchmark 对比只能作为本机工程信号，不是语义测试，
也不是跨 runtime 的绝对排名。

不要把 per-item native call 和 batched JavaScript loop 对比；那主要测的是 FFI
overhead。应比较相近规模的 batch call。

WASM unchecked benchmark 结果不代表 checked arithmetic 安全性。Unchecked WASM 可用于
portability 和 host integration 实验，但它不会报告 integer overflow、division by
zero safety、pointer validity 或 buffer length 错误。

拆解后的 WASM case 用来分离可能的瓶颈层：

- `pricing-wasm-unchecked-total`：在被测 workload 中写 memory、调用
  `calc_items`、再读取 checksum。
- `pricing-wasm-unchecked-compute-only`：只预写一次 memory，之后重复调用
  `calc_items`，最后读取一次 checksum。
- `pricing-wasm-unchecked-memory-only`：只测 host 侧 `DataView` memory
  write/read，不调用 WASM。
- `pricing-wasm-unchecked-call-overhead`：重复调用一个极小 generated WASM
  function，用来估算 JS-to-WASM 边界成本。
