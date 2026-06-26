# IntKernel 性能画像

日期：2026-06-24

本文记录 Phase 14 优化前的 pricing benchmark 性能画像。目标是先确认瓶颈，
不修改编译器优化逻辑。

## Benchmark 背景

最近一次 full run：

- Items：100000
- 每个命令内部 iterations：1000
- Hyperfine runs：20
- Warmup runs：3
- 平台：macOS arm64
- CPU：Apple M4 Pro
- Node：v24.14.0
- Clang：Apple clang 17.0.0

| Case | Category | Opt | Mode | Median ms | vs C O3 |
| --- | --- | --- | --- | ---: | ---: |
| pricing-c-unchecked-O0 | native | O0 | unchecked | 621.854 | 10.77x |
| pricing-c-unchecked-O2 | native | O2 | unchecked | 58.140 | 1.01x |
| pricing-c-unchecked-O3 | native | O3 | unchecked | 57.754 | 1.00x |
| pricing-c-checked-O3 | native | O3 | checked | 80.968 | 1.40x |
| pricing-llvm-unchecked-O0 | native | O0 | unchecked | 615.368 | 10.66x |
| pricing-llvm-unchecked-O3 | native | O3 | unchecked | 57.779 | 1.00x |
| pricing-wasm-unchecked-total | wasm | n/a | unchecked | 2699.638 | 46.74x |
| pricing-wasm-unchecked-compute-only | wasm | n/a | unchecked | 220.336 | 3.82x |
| pricing-wasm-unchecked-memory-only | memory | n/a | unchecked | 4754.650 | 82.33x |
| pricing-wasm-unchecked-call-overhead | call-overhead | n/a | unchecked | 24.090 | 0.42x |
| pricing-js-number | js | n/a | host | 263.457 | 4.56x |
| pricing-js-typedarray-number | js | n/a | host | 123.421 | 2.14x |
| pricing-js-bigint | js | n/a | host | 182.220 | 3.16x |

## 轻量 profiling 证据

本机可用 `time`。它对最快 native command 的 quick scale 粒度太粗，但能确认大体趋势：

- `pricing-c-unchecked-O3`，100000 items x 100 iterations：低于 `time -p`
  的可见分辨率。
- `pricing-wasm-unchecked-compute-only`，相同规模：约 0.05 秒。
- `pricing-wasm-unchecked-memory-only`，100000 items x 10 iterations：约
  0.12 秒。
- `pricing-js-bigint`，100000 items x 100 iterations：约 0.04 秒。

`sample` 位于 `/usr/bin/sample`，也可以手动使用 Instruments 做更深入的 native 或
Node profiling。它们是可选工具，不是 benchmark runner 的硬依赖。

LLVM 侧使用 clang 查看优化后的 IR：

```sh
clang -O3 -S -emit-llvm build/perf/generated/pricing.ll \
  -o build/perf/generated/pricing.optimized.ll
```

未优化的 generated LLVM IR 有 22 个 `alloca`、36 个 `load` 和 24 个 `store`。
经过 clang `-O3` 后，hot function 中 alloca 被消掉，只剩 4 次从 `items` 的 load、
1 次写 `out` 的 store，以及带 phi induction variable 的紧凑 loop。这说明 clang
能把当前 alloca/load/store lowering 优化成合理形态。

## 为什么 C unchecked 最快

Unchecked C 是当前最好的 native hot path：

- 生成直接整数运算和直接 struct field load/store。
- Clang `-O2` 和 `-O3` 都能把 MIR 风格 temps 和 goto 优化成紧凑 loop。
- 没有 status propagation、overflow branch 或 host 边界成本。
- 整个 batch 都在一次 native call 内完成。

本 benchmark 中 `O2` 和 `O3` 基本持平。`O0` 慢约 10.8 倍，说明原始生成 C 是正确的，
但 hot path 性能强依赖 C compiler optimizer。

## C checked 慢在哪里

Checked C full run median 约为 unchecked C O3 的 1.40 倍。

生成的 checked loop 除了实际 pricing arithmetic，还包含：

- 两次 `__builtin_mul_overflow`
- 一次 `__builtin_sub_overflow`
- 两次 `__builtin_add_overflow`，包括 `i + 1`
- 对常量 divisor `1000000` 的 division-by-zero branch
- signed min / `-1` overflow branch
- 每个 checked operation 的 early return branch
- checked ABI 的 return pointer null check 和最终 `ck_return` 写入

部分检查是语义必须的。部分在当前代码中可证明冗余，例如 literal `1000000` 的 zero
检查，但只能通过通用且有 correctness test 的 pass 删除。

## WASM 慢在哪里

WASM 目前慢在两个不同层面：

1. **Host memory marshaling 很贵**。`pricing-wasm-unchecked-memory-only` 明显最慢，
   因为它反复通过 `DataView` 写 `Item` fields 并读取 checksum。
2. **WASM compute-only 仍然慢**。WAT 使用 block dispatcher：

   - `ik_bb` local
   - `br_table` dispatcher loop
   - 嵌套 block cases
   - 大量 `local.get` / `local.set`

   每次源码 loop iteration 都回到 dispatcher，而不是使用结构化 WebAssembly loop。
   loop body 还为每个 field load 反复计算 `items + i * 32`。

极小函数 call-overhead case 很快，所以 JS-to-WASM 边界成本不是 pricing 慢于 JS BigInt
的主要原因。它仍然会影响细粒度 API，但当前 benchmark 是批量调用。

## JS BigInt 慢在哪里

JS BigInt baseline 精确保留 `i64` 风格 arithmetic，但有这些成本：

- BigInt multiply/subtract/add/divide
- `BigInt64Array` element conversion
- BigInt checksum accumulation

它比 Number typed-array case 慢，但比当前 WASM compute-only 快。这说明当前 WASM code
shape 是主要瓶颈之一，而不是单纯因为 i64 arithmetic。

普通 Number array 比 TypedArray Number 更慢，说明 JS 数据布局对该 benchmark 影响明显。

## Benchmark harness 开销

拆解 case 已经把 harness 成本显式分开：

- `total`：包含 host memory write、compute、checksum read。
- `compute-only`：预写 memory，只重复调用 `calc_items`，最后读取一次 checksum。
- `memory-only`：隔离 host `DataView` memory work。
- `call-overhead`：隔离极小 JS-to-WASM call loop。

优化 backend 时应主要看 `compute-only`。做 host 集成建议时，`total` 和 `memory-only`
仍然重要。

## Generated C 审查

当前 unchecked generated C 有很多临时变量和 goto label：

- `ik_tmp0` 到 `ik_tmp14`
- 显式 `goto bb1`、`bb2`、`bb3`
- 对 `0`、`1`、`1000000` 这类 literal 的临时赋值

Clang `-O3` 能很好处理它们。但 MIR 层 cleanup 仍然有价值：改善可读性、降低 O0 成本，
也减少对后端 optimizer 的依赖。

潜在优化：

- 删除 single-use move temp
- 将 literal temp fold 到使用点
- 简化 loop induction update
- 在安全时复用重复 pointer/index address
- 只有在语言有 sound aliasing contract 后，才考虑暴露 alias 信息

不要在没有语言级 non-aliasing 规则时默认添加 `restrict`。

## Generated checked C 审查

Checked mode 正确保留安全性，但 pricing loop 中有些检查可通过通用 optimizer 证明冗余：

- literal `1000000` 不可能为 0
- literal `1000000` 不可能为 `-1`
- return literal `0` 不需要 arithmetic check

`i + 1` overflow check 语义上是合理的。要删除它，需要 range analysis 证明
`i < len`、`len <= INT32_MAX`，并证明递增不会溢出；目前还没有这个分析。

## Generated WAT 审查

WAT 形态是当前最大 backend 问题：

- 多 block function 使用 dispatcher，而不是结构化 loop。
- 每次 loop iteration 都更新 `ik_bb` 并走 `br_table`。
- 大量 `local.get` / `local.set`。
- price、qty、discount、tax rate 每次都重复计算 `items + i * 32`。
- host 侧 `DataView` 读写主导 total/memory-only case。

最高收益的 WASM 优化是把可结构化 pattern 输出为原生 WebAssembly `loop` / `block`。

## Generated LLVM 审查

LLVM v1 生成 alloca/load/store 形式，简单正确但啰嗦。pricing 经过 clang `-O3` 后，
alloca-heavy 形态被消掉，生成紧凑 loop。因此 LLVM O3 和 C O3 基本持平。

未来可优化：

- 对简单 expression/loop 做直接 SSA-like lowering
- 为 load/store 和 GEP pointer 输出 alignment 信息
- `-O0` 保留 alloca lowering 以便调试

由于 clang 已经能很好优化 pricing kernel，LLVM backend 不是 Phase 14 的第一优先级。

## 优化优先级

| 优先级 | 项目 | 预期收益 | 说明 |
| --- | --- | --- | --- |
| P0 | MIR pass manager 和 opt-level plumbing | 基础设施必须做 | 后续所有优化都需要 `-O0` 关闭开关和 validator 集成。 |
| P0 | 保持 benchmark 拆解信号 | 已开始 | 避免误把 host-memory mixed benchmark 当作 backend compute 瓶颈。 |
| P1 | WASM structured loop/control-flow emission | 高 | 直接针对 compute-only WASM 约 3.8x C O3 的问题。 |
| P1 | MIR temp/move/literal cleanup | 高 | 改善 C O0、WAT、LLVM O0、可读性和后续 passes。 |
| P1 | Common address calculation cleanup | 高 | pricing 反复计算 `items + i * sizeof(Item)`。 |
| P1 | Checked constant divisor simplification | 中高 | 删除 literal divisor 的冗余 div-zero 和 min/-1 checks。 |
| P2 | Checked arithmetic range analysis | 中 | 可能删除 loop-index overflow check，但需要证明基础设施。 |
| P2 | LLVM direct SSA-like lowering | 中 | 长期有用；当前 clang 已能恢复 pricing O3。 |
| P2 | LLVM alignment metadata | 中 | 当前优化后 IR 对 i64 load/store 仍偏保守。 |
| P3 | Host memory marshaling redesign | 暂缓 | 更像 API/使用方式或未来 memory helper，不是纯 compiler 优化。 |
| P3 | `restrict` 或 noalias 假设 | 暂缓 | 需要 sound language-level aliasing contract。 |
| P3 | SIMD/threading/PGO | 不做 | Phase 14 明确排除。 |

## 建议下一步

Phase 14.4 先做基础设施，不直接做优化：

1. 新增 `OptimizationLevel` 和 CLI plumbing。
2. 新增 MIR pass manager，`O0` 为 no-op。
3. 每个 pass 后运行 MIR validator。
4. 先增加 pass-level tests 和 snapshots，再改变 backend 输出。

然后再做 safe MIR cleanup passes 和 WASM structured loop emission。
