# 性能

[English](../PERFORMANCE.md)

本文总结 Phase 22 本机 performance suite、当前本机结果，以及如何运行性能回归检查。这些数字
只代表本机测量结果；不要跨机器比较绝对耗时。结果依赖硬件、Node.js、clang、
hyperfine、OS scheduling、电源状态和当前系统负载。
Release-facing 口径见 [v0.8.0 release notes](../releases/v0.8.0.md)。

## Benchmark Suite

基于 hyperfine 的套件位于 `bench/perf`，主要针对 `examples/pricing.ck`、一个
helper-function fixture，以及第一版 strict `f64` compute kernel。当前覆盖：

- native C unchecked：`pricing-c-unchecked-O0`、`pricing-c-unchecked-O2`、
  `pricing-c-unchecked-O3`、`pricing-c-unchecked-ck-O3`
- native C checked：`pricing-c-checked-O3`
- LLVM unchecked：`pricing-llvm-unchecked-O0`、`pricing-llvm-unchecked-O2`、
  `pricing-llvm-unchecked-O3`
- WASM unchecked total 和 compute-only，分别覆盖 `CK-O0` 和 `CK-O3`
- WASM memory-only 和 JS-to-WASM call-overhead 拆解
- WASM pricing SoA resident-memory case，使用 exported memory 上的
  `BigInt64Array` view
- JavaScript baseline：`Number`、typed-array `Number`、`BigInt`
- f64 kernel：axpy、dot product、sum、scale
- f64 对比目标：JavaScript `Array` `Number`、JavaScript `Float64Array`、
  CK C O3、CK LLVM O3、CK WASM O3
- f64 WASM total、compute-only 和 memory-only 拆解
- f64 WASM low-copy 变体，使用 exported memory 上的 `Float64Array` view

标准 pricing workload：

- 100,000 items
- 1,000 次 `calc_items` iteration
- 每个 case 都校验 checksum

标准 f64 workload 使用 deterministic `Float64` input，消费每个结果 checksum，并用
absolute tolerance 加 relative tolerance 校验。它不要求 C、LLVM、WASM 和 JavaScript
之间的浮点结果 bit-identical。

## 运行 Benchmark

Benchmark 是手动 release 或开发工具。它们故意不属于普通 `pnpm test`，性能阈值
也不能被移入 unit test suite。

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
node bench/perf/run.mjs --full --case pricing-wasm-soa
node bench/perf/run.mjs --quick --case f64
node bench/perf/run.mjs --quick --case f64-axpy
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

不要把本机 baseline 当成跨机器契约。只有在明确机器和 toolchain context 的显式本机
性能运行中，才使用 `--compare` 和 `--fail-on-regression`。

## 当前 Full Run 摘要

本机最新 Phase 22 full run，2026-06-26：

| Case | Median ms | 解读 |
| --- | ---: | --- |
| `pricing-js-typedarray-number` | 123.519 | JS typed-array `Number` baseline |
| `pricing-js-bigint` | 181.427 | 精确 JS `BigInt` baseline |
| `pricing-wasm-unchecked-compute-only-O3` | 118.215 | 预写 memory 后的 WASM compute path |
| `pricing-wasm-unchecked-total-O3` | 2562.414 | DataView AoS fallback total |
| `pricing-wasm-soa-setup-copy-in-O3` | 30.465 | SoA resident 一次性 copy-in |
| `pricing-wasm-soa-resident-total-O3` | 111.615 | 推荐 SoA resident path |
| `pricing-wasm-soa-readback-cost-O3` | 261.313 | 重复 output readback 成本 |
| `pricing-wasm-soa-total-with-final-readback-O3` | 118.219 | resident compute 加一次最终 readback |

| Case | Median ms | 解读 |
| --- | ---: | --- |
| `f64-sum-js-float64array` | 112.375 | JS `Float64Array` baseline |
| `f64-sum-ck-wasm-o3-compute-only` | 90.739 | WASM compute path |
| `f64-sum-ck-wasm-o3-optimized-low-copy-total` | 89.150 | 推荐 resident/scalar-return path |
| `f64-sum-ck-wasm-o3-total` | 1033.140 | DataView fallback total |
| `f64-axpy-js-float64array` | 114.551 | JS `Float64Array` baseline |
| `f64-axpy-ck-wasm-o3-compute-only` | 99.539 | WASM compute path |
| `f64-axpy-ck-wasm-o3-view-output-total` | 114.269 | 推荐 output-view path |
| `f64-axpy-ck-wasm-o3-copy-output-total` | 204.850 | 显式 copy-output path |
| `f64-axpy-ck-wasm-o3-total` | 1147.689 | DataView fallback total |

这些表格不表示“CK WASM 普遍比 JS 快”。它们说明 CK WASM compute path 已有竞争力；
在 resident memory、SoA layout、typed-array bulk copy、scalar return 和 output
view 这些条件成立时，部分批量 workload 有机会快于对应 JavaScript typed-array
baseline。mixed-width struct 的 `DataView` marshal 和大量 output copy/readback 会让
WASM total 明显变慢。

## Backend 对比

Native unchecked C 是参考基线。对 pricing kernel 来说，clang `-O2` 和 `-O3`
结果基本相同。

当前 run 中 checked C 比 unchecked C 慢约 40%。这些开销来自 overflow check、
division check 和 status-return control flow。checked backend 保留 `price * qty`
这类业务算术检查；`-O3` 只会消除已证明安全的 loop induction increment。

LLVM `-O2` 和 `-O3` 对 `pricing.ck` 基本追平 native C。通用 LLVM 函数仍使用
alloca/load/store lowering，但 clang 能很好地提升 hot path。简单 scalar
straight-line function 在 `-O2` 和 `-O3` 下可以使用小型 SSA-like lowering。

WASM 在 compute-only mode 下经过 simple while-loop structured lowering 和 indexed
address reuse 后有明显改善，但仍慢于 native C 和 LLVM。WASM total case 主要被 host
侧 `DataView` memory setup 和 checksum read 拖慢，而不是 JS-to-WASM call overhead。

对 pricing，Phase 22 新增推荐的 SoA resident-memory benchmark fixture：

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

这个 fixture 保持 `calc_items` 的整数 fixed-point 算术语义，但把输入拆成
homogeneous arrays。JavaScript 侧用 `BigInt64Array#set` 一次性 bulk-copy 到 WASM
memory，让 output 留在 WASM memory，并单独报告 readback cost。这是大批量 resident
pricing 数据的推荐 interop 形态。原 mixed-width/AoS `DataView` 路径继续作为
fallback/debug ABI 对照。

JavaScript `BigInt` 适合作为精确 `i64` baseline，但慢于 native C 和 LLVM。
Typed-array `Number` 比 `BigInt` 快，但不能对所有值提供精确 `i64` 语义。

这套 benchmark 不是 NumPy 级或 vectorized-library 性能保证，也不能把 WASM 泛化为
JavaScript typed-array hot loop 的替代品。WASM total time 可能主要受 host memory
marshaling 影响。

C、LLVM 构建出的 native binary、WASM、JavaScript，以及任何可选 Python harness
并不共享同一种 runtime model。它们的对比只用于理解 workload shape、boundary cost
和 safety tradeoff；不要把这些结果当作语言语义测试，也不要当作跨 runtime 的绝对
排名。

对 f64 kernel，JavaScript `Array` `Number`、JavaScript `Float64Array`、CK C、
CK LLVM、CK WASM、可选 Python list `float` 和可选 NumPy 属于不同 runtime model。
NumPy 是 native-library baseline，不是默认 runner dependency。f64 suite 只假设
strict CK floating point：`f64` 是唯一 floating point type，不规划 `f32`，也不假设
fast-math、SIMD、implicit int/float conversion、broad cast 或 f64 checked
overflow。当前唯一 numeric cast 是 exact explicit `i32_to_f64` 和 `u32_to_f64`
builtin。JavaScript `Float64Array` 可以是很强的 host 热循环 baseline。WASM
compute-only 和 WASM total 回答的是不同问题，所以必须先确认测量的是哪个 phase，
再判断 WASM 与 JavaScript 的相对表现。

## Batch Calling 原则

应该 benchmark 并发布批量调用：

```ck
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

f64 WASM benchmark 暴露更细的 Phase 18.1 拆分：

- setup：实例化 f64 WASM module 并准备 linear memory
- input-marshal：把 deterministic `f64` 输入写入 WASM memory
- compute-only：预写入 memory 后重复调用 WASM kernel
- output-readback：从 WASM memory 读回计算后的 `f64` buffer
- total：input marshal + WASM compute + output readback 的同一条测量路径
- memory-only：host 侧写入 + 读回，不执行 WASM kernel

生成的 summary 包含 `Phase` 列，便于在 `build/perf/latest.summary.md` 中观察这些
路径。f64 参数和返回值使用 JavaScript `Number`；pricing 的 i64/u64 path 仍在需要时
使用 `BigInt`。Phase 18.2 增加 `examples/node-wasm-f64-array/` 作为批量
`ptr<f64>` buffer 的推荐 host 模式：在 exported WASM memory 上创建
`Float64Array` view，用 `byteOffset / 8` 转换 offset，并在 hot path 使用
typed-array bulk operation，而不是逐元素 `DataView` 调用。`DataView` 仍是
mixed-width struct 检查所需的 byte-level ABI 工具。

Phase 18.3 增加 `f64-*-ck-wasm-o3-low-copy-*` benchmark case。这些 case 不改变
WASM pointer ABI，而是单独测推荐 host path：用 `Float64Array#set` 做 input
marshal，执行 WASM compute，`dot`/`sum` 消费 scalar return，in-place array kernel
只读回 output checksum。原 DataView case 继续保留，用于观察 byte-level marshal
开销。

Phase 22 增加基于 CKWasmArena 的 f64 optimized benchmark case，覆盖两个主要
JS/WASM interop 形态：

- `f64-sum-ck-wasm-o3-optimized-low-copy-total` 先用 `Float64Array#set` 把
  input 拷入 WASM memory，并让数据 resident，随后重复调用 strict `sum_f64`
  kernel，消费 scalar `f64` return，不做 output readback。这是 resident
  homogeneous `f64` reduction 的推荐 CK WASM 形态。
- `f64-axpy-ck-wasm-o3-view-output-total` 只把 resident `x` input 拷贝一次，
  每轮用 `Float64Array#set` 刷新 `y`/output，让 WASM kernel 把 output 写入
  WASM memory，并以 WASM memory view 作为 output。这是 in-place/output buffer
  场景的推荐形态。
- `f64-axpy-ck-wasm-o3-copy-output-total` 显式把 output 拷贝成 JS-owned
  `Float64Array`。当 ownership 需要时使用它，但大量 copy-out 会削弱甚至抵消
  WASM 优势。

production-style homogeneous f64 buffer 优先使用 CKWasmArena low-copy /
view-output path。需要检查 byte offset、mixed-width struct 和 ABI 精确性时，
继续使用 DataView。DataView total case 继续作为 fallback 对照，而不是 large f64
buffer 的推荐路径。

Phase 22 也新增基于 CKWasmArena 的 pricing SoA case：

- `pricing-wasm-soa-setup-copy-in-O3` 测量 resident pricing arrays 的一次性
  arena allocation、memory growth 和 `BigInt64Array#set` copy-in。
- `pricing-wasm-soa-resident-total-O3` 只 copy input 一次，重复调用
  `pricing_soa`，检查 scalar `i32` status return，并让 output 保持 WASM memory view。
- `pricing-wasm-soa-readback-cost-O3` 隔离重复 `BigInt64Array` output view
  checksum/readback 成本。
- `pricing-wasm-soa-total-with-final-readback-O3` 测量 resident compute 加一次最终
  output view checksum。

当 JavaScript 能把数据保持为 homogeneous typed arrays 时，pricing workload 优先使用
SoA + resident memory。不要把 `DataView` pricing total 当成推荐性能路径；它用于保留
mixed-width struct ABI 成本的可见性。

推荐 interop 形态的官方可运行 examples 位于 `examples/wasm`：

- [`examples/wasm/f64-sum`](../../examples/wasm/f64-sum/README.md)：read-only
  `Float64Array` input，scalar `f64` return，不做 output readback。
- [`examples/wasm/f64-axpy`](../../examples/wasm/f64-axpy/README.md)：output view
  fast path，`copyOutF64` 只作为显式 JS-owned copy 演示。
- [`examples/wasm/pricing-soa`](../../examples/wasm/pricing-soa/README.md)：SoA
  integer fixed-point pricing，使用 WASM memory 上的 `BigInt64Array` view。

构建后运行：

```sh
pnpm build
node examples/wasm/f64-sum/run.mjs
node examples/wasm/f64-axpy/run.mjs
node examples/wasm/pricing-soa/run.mjs
```

`memory.grow` 后必须重新创建 `Float64Array` view；CK 不提供 WASM allocator 或
runtime，memory placement 和 buffer sizing 仍由 host 负责。

当前 WASM 最大瓶颈：total benchmark 中的 host-side memory setup/readback。
compute-only WASM 已接近很多，但仍慢于 native code。

f64 benchmark 解读锁定为 strict semantics：

- quick run 是 smoke check，不是 release performance claim
- full run 是可选手动 release check
- 不把 f64 性能阈值放入普通 `pnpm test`
- 不提交本机 f64 baseline
- 有限 f64 结果使用 absolute tolerance 和 relative tolerance 对比
- NaN、infinity 和 `-0.0` 用分类判断，不要求 bit-identical output
- JS `Array` `Number`、JS `Float64Array`、WASM、native C、LLVM、可选 Python
  和可选 NumPy 都是不同 runtime model
- 分开解读 DataView total 和 low-copy total；两者都不是跨机器性能保证

## Checked vs Unchecked

输入已被证明安全且最大吞吐最重要时，使用 unchecked mode。金额、税费、优惠或规则
计算需要算术安全时，使用 checked C mode。

Checked mode 当前只适用于 C 输出。WASM 和 LLVM backend 会拒绝
`--overflow checked`，不会静默生成 unchecked code。

## 当前最大瓶颈

1. WASM DataView total benchmark：host `DataView` memory setup 和 checksum
   readback。
2. Pricing 和 f64 copy-out/readback：大量 output read 会抵消 WASM compute-only
   优势。
3. WASM compute-only：generated WAT/VM 执行仍慢于 native C O3。
4. Checked C：业务 overflow checks 仍必要，成本约 40%。
5. LLVM O0：不打开 clang optimization 时，stack lowering 按预期较慢。

最有价值的后续方向：

- 更广泛的 WASM structured control-flow lowering
- pricing interop 优先使用 SoA resident-memory，而不是 mixed-width AoS DataView
  hot path
- 对更多无 memory 的 scalar control flow 做 direct SSA LLVM lowering
- 在默认关闭的前提下实验 CPU-native/LTO 等显式 unsafe/target-specific 选项
