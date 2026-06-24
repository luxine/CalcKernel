# Pricing Benchmark Harness

[English](README.md)

本目录包含针对 `examples/pricing.ik` 的小型 benchmark harness，用于对比纯
JavaScript baseline、生成的 native C、生成的 checked C，以及生成的 unchecked
WASM。它们只是粗略的本地参考，不是稳定的 CI 性能套件。

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
会使用更多 hyperfine samples 和更多内部循环。

runner 会完成完整本机准备工作：

1. 运行 `pnpm build`
2. 生成 unchecked C、checked C 和 unchecked WASM 到 `build/perf/generated`
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

本机 baseline 保存在 `build/perf/baseline.local.json`，不应提交。不要跨机器比较
绝对性能数字。

第一版套件包含：

- `pricing-c-unchecked`
- `pricing-c-checked`
- `pricing-wasm-unchecked`
- `pricing-js-bigint`

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

不要把 per-item native call 和 batched JavaScript loop 对比；那主要测的是 FFI
overhead。应比较相近规模的 batch call。

WASM unchecked benchmark 结果不代表 checked arithmetic 安全性。Unchecked WASM 可用于
portability 和 host integration 实验，但它不会报告 integer overflow、division by
zero safety、pointer validity 或 buffer length 错误。
