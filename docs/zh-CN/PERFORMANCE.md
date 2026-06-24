# 性能

[English](../PERFORMANCE.md)

本文总结 Phase 14 本机 performance suite、当前本机结果，以及如何运行性能回归检查。
这些数字只代表本机测量结果；不要跨机器比较绝对耗时。

## Benchmark Suite

基于 hyperfine 的套件位于 `bench/perf`，主要针对 `examples/pricing.ik` 和一个
helper-function fixture。当前覆盖：

- native C unchecked：`pricing-c-unchecked-O0`、`pricing-c-unchecked-O2`、
  `pricing-c-unchecked-O3`、`pricing-c-unchecked-ik-O3`
- native C checked：`pricing-c-checked-O3`
- LLVM unchecked：`pricing-llvm-unchecked-O0`、`pricing-llvm-unchecked-O2`、
  `pricing-llvm-unchecked-O3`
- WASM unchecked total 和 compute-only，分别覆盖 `IK-O0` 和 `IK-O3`
- WASM memory-only 和 JS-to-WASM call-overhead 拆解
- JavaScript baseline：`Number`、typed-array `Number`、`BigInt`

标准 workload：

- 100,000 items
- 1,000 次 `calc_items` iteration
- 每个 case 都校验 checksum

## 运行 Benchmark

快速本机 smoke：

```sh
node bench/perf/run.mjs --quick
```

完整本机运行：

```sh
node bench/perf/run.mjs --full
```

只运行部分 case：

```sh
node bench/perf/run.mjs --quick --case pricing-c-unchecked
node bench/perf/run.mjs --full --case pricing-llvm-unchecked --case pricing-wasm-unchecked-compute-only
```

## Baseline 和回归检查

保存本机私有 baseline：

```sh
node bench/perf/run.mjs --full --save-baseline
```

和 baseline 比较：

```sh
node bench/perf/run.mjs --full --compare
node bench/perf/run.mjs --full --compare --threshold 10 --fail-on-regression
```

回归检查基于 median runtime。comparison report 包含当前 median、baseline median、
runtime ratio 和变慢百分比。

Baseline 策略：

- 真实本机 baseline 写入 `build/perf/baseline.local.json`。
- `build/` 已经被 git 忽略；不要提交开发机器上的 baseline。
- `bench/perf/baselines/example.summary.json` 只是格式示例。
- 普通 `pnpm test` 不运行 hyperfine，也不会因为性能波动失败。

## 当前 Full Run 摘要

本机最新 Phase 14 full run，2026-06-24：

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

## Backend 对比

Native unchecked C 是参考基线。对 pricing kernel 来说，clang `-O2` 和 `-O3`
结果基本相同。

当前 run 中 checked C 比 unchecked C 慢约 40%。这些开销来自 overflow check、
division check 和 status-return control flow。checked backend 保留 `price * qty`
这类业务算术检查；`-O3` 只会消除已证明安全的 loop induction increment。

LLVM `-O2` 和 `-O3` 对 `pricing.ik` 基本追平 native C。通用 LLVM 函数仍使用
alloca/load/store lowering，但 clang 能很好地提升 hot path。简单 scalar
straight-line function 在 `-O2` 和 `-O3` 下可以使用小型 SSA-like lowering。

WASM 在 compute-only mode 下经过 simple while-loop structured lowering 和 indexed
address reuse 后有明显改善，但仍慢于 native C 和 LLVM。WASM total case 主要被 host
侧 `DataView` memory setup 和 checksum read 拖慢，而不是 JS-to-WASM call overhead。

JavaScript `BigInt` 适合作为精确 `i64` baseline，但慢于 native C 和 LLVM。
Typed-array `Number` 比 `BigInt` 快，但不能对所有值提供精确 `i64` 语义。

## Batch Calling 原则

应该 benchmark 并发布批量调用：

```ik
export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32
```

不要每个 item 跨一次 FFI 或 JS-to-WASM 边界。如果逐条调用，boundary cost 和 host
memory marshaling 很容易成为主导。

## WASM 注意事项

当前 WASM backend 只支持 unchecked，不提供 runtime、allocator 或 bounds check。
Host code 负责 memory layout、`DataView` little-endian 写入和 output buffer 大小。

做性能分析时应拆分：

- total time：host memory setup + WASM compute + checksum read
- compute-only time：预写 memory + 重复 WASM call
- memory-only time：host 侧 `DataView` 工作
- call-overhead time：JS-to-WASM 边界开销

当前 WASM 最大瓶颈：total benchmark 中的 host-side memory setup/readback。
compute-only WASM 已接近很多，但仍慢于 native code。

## Checked vs Unchecked

输入已被证明安全且最大吞吐最重要时，使用 unchecked mode。金额、税费、优惠或规则
计算需要算术安全时，使用 checked C mode。

Checked mode 当前只适用于 C 输出。WASM 和 LLVM backend 会拒绝
`--overflow checked`，不会静默生成 unchecked code。

## 当前最大瓶颈

1. WASM total benchmark：host `DataView` memory setup 和 checksum readback。
2. WASM compute-only：generated WAT/VM 执行仍约为 native C O3 的 2 倍。
3. Checked C：业务 overflow checks 仍必要，成本约 40%。
4. LLVM O0：不打开 clang optimization 时，stack lowering 按预期较慢。

最有价值的后续方向：

- 更广泛的 WASM structured control-flow lowering
- 降低示例和 benchmark 中 WASM i64 memory marshaling overhead
- 对更多无 memory 的 scalar control flow 做 direct SSA LLVM lowering
- 在默认关闭的前提下实验 CPU-native/LTO 等显式 unsafe/target-specific 选项
