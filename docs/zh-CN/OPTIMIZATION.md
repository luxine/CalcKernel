# 优化

[English](../OPTIMIZATION.md)

IntKernel 暴露统一的 compiler optimization level，供 MIR pass 在 C、WASM、
LLVM 和 MIR 调试输出之间一致接入。

Phase 14.4 接入选项 plumbing。Phase 14.5 新增 MIR pass manager 框架。
Phase 14.6 新增保守的局部 MIR pass。Phase 14.7 新增 CFG simplification。
Phase 14.8 到 Phase 14.10 新增 local CSE、address CSE、小函数内联和保守
loop analysis。Phase 14.12 为已证明安全的 loop induction increment 增加
checked C 热路径优化。Phase 14.13 为 WASM 增加 simple while loop 热路径 lowering
和 indexed address reuse。Phase 14.14 为 LLVM 增加 build optimization flag 透传，
并为简单 scalar straight-line function 增加小型 SSA-like lowering 路径。

## CLI

所有 code generation 命令都接受 `--opt-level`：

```sh
ikc emit-mir examples/pricing.ik --opt-level 0
ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h --opt-level 3
ikc build examples/pricing.ik --out build/libpricing --opt-level 3
ikc emit-wat examples/pricing.ik --out build/pricing.wat --opt-level 3
ikc emit-wasm examples/pricing.ik --out build/pricing.wasm --opt-level 3
ikc emit-llvm examples/pricing.ik --out build/pricing.ll --opt-level 3
ikc build-llvm examples/pricing.ik --out build/libpricing --opt-level 3
```

`-O` alias 等价：

```sh
ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h -O3
```

默认值是 `-O0`。

## 等级

### `-O0`

不做 MIR optimization。

`-O0` 只运行 validator。它让 compiler 输出最接近 lowered MIR，也是 Phase 14
的默认值。它是调试和 snapshot review 的基线。

### `-O1`

低成本局部优化。

`-O1` 当前运行：

- constant folding
- copy propagation
- dead code elimination（DCE）
- simple CFG cleanup，但只删除 unreachable block

### `-O2`

标准优化管线。

`-O2` 当前运行 `-O1`，并额外启用：

- 完整 CFG simplification
- local CSE
- address CSE
- 使用 O2 threshold 的 small-function inlining
- inlining 和 CSE 后重复运行 cleanup pass

### `-O3`

激进优化管线。

`-O3` 启用 `-O2` pipeline，并额外启用：

- 使用 O3 threshold 的更激进 small-function inlining
- 保守 basic loop analysis
- 保守 loop-invariant code motion
- induction simplification metadata 和 checked C induction proof support
- C 和 LLVM native build command 使用 clang `-O3`
- simple while loop 的 WASM hot-path lowering

可选的 CPU-native 和 LTO 控制保留给未来工作。它们默认不启用，Phase 14 也不添加
unsafe target-specific flag。`-O3` 仍然必须保留所选择的 overflow mode、ABI 和语言
语义。

## Pass Manager

MIR pass manager 是共享 optimization 入口。它运行在 MIR lowering 之后、
backend emission 之前：

```text
CheckedProgram
  -> MIR lowering
  -> MIR pass manager
  -> MIR validator
  -> backend
```

每个 pass 实现：

```ts
interface MirPass {
  name: string;
  run(module: MirModule, context: MirPassContext): MirPassResult;
}
```

pass context 携带：

- optimization level
- overflow mode
- target backend
- debug flags

pass result 会报告该 pass 是否改变 MIR，也可以包含 diagnostics。manager 会记录
pass 顺序和 changed 状态，并且可以在每个 pass 后运行 MIR validator。

当前 pipeline：

- `-O0`：只运行 validator
- `-O1`：constant folding -> copy propagation -> dead code elimination -> CFG
  simplification
- `-O2`：constant folding -> copy propagation -> local CSE -> copy
  propagation -> address CSE -> dead code elimination -> CFG simplification ->
  dead code elimination
- `-O3`：constant folding -> copy propagation -> small-function inlining ->
  constant folding -> copy propagation -> loop analysis -> loop-invariant code
  motion -> induction simplification -> constant folding -> copy propagation ->
  local CSE -> copy propagation -> address CSE -> dead code elimination -> CFG
  simplification -> dead code elimination

## Passes

### Constant Folding

constant folding pass 会在 unchecked MIR 中折叠纯常量表达式。它支持整数
`+`、`-`、`*`、`/`、`%`、comparison、unary `-`、unary `!` 和 bool 常量。

安全边界：

- checked overflow mode 下禁用
- 不折叠除零的除法或取模
- 不折叠 signed min / -1 的除法或取模
- 不折叠结果超出目标整数类型范围的整数运算
- 不折叠 memory access、store、call 或 control-flow effect

### Copy Propagation

copy propagation pass 会在 basic block 内替换简单 temp copy。

安全边界：

- 不传播 lvalue place
- 不跨越 `store`
- 不跨越 `call`
- 只重写 MIR value use 和 index expression

### Dead Code Elimination

dead code elimination pass 会删除未使用的纯 temp definition。

可以删除未使用的：

- `const_int`
- `const_bool`
- 到 temp 的纯 `move`
- 纯 `binary`
- 纯 `unary`
- 纯 `compare`

不会删除：

- `store`
- `call`
- `load`
- terminator
- branch condition
- return value
- local assignment

### Local CSE

local common subexpression elimination pass 只在单个 basic block 内工作。

可以复用：

- 纯 binary arithmetic
- 纯 unary expression
- comparison

安全边界：

- 不做 global CSE
- 不对普通 load 做 CSE
- 遇到 `store` 和 `call` 清空表
- 依赖的 local 被重新赋值时失效相关表达式
- 依赖后续 copy propagation 和 DCE 清理替换产生的 move

### Address CSE

address CSE pass 当前只在 C backend 的 `-O2` 和 `-O3` 下启用。它识别同一个
basic block 内的 indexed place，并把 address 提升成 pointer temp。这包括重复的
`ptr<Struct>[i].field` 读取，以及 `out[i] = value` 这样的 scalar indexed store。

示例生成 C 形态：

```c
Item* ik_tmp_addr0;

ik_tmp_addr0 = &items[i];
ik_tmp0 = ik_tmp_addr0->price;
ik_tmp1 = ik_tmp_addr0->qty;
```

scalar store 示例：

```c
int64_t* ik_tmp_addr1;

ik_tmp_addr1 = &out[i];
(*ik_tmp_addr1) = ik_tmp11;
```

安全边界：

- 不消除或缓存 load
- 不做 alias analysis
- 遇到 `store` 和 `call` 清空 address 状态
- 依赖的 local 被重新赋值时失效相关 address entry
- 不针对 `Item` 或 `pricing.ik` 做特判

### WASM Hot Path Lowering

WASM backend 对任意 MIR CFG 仍保留原来的 block-dispatcher fallback。在 `-O3`
下，它会识别简单 while-loop 形态：

```text
entry -> condition
condition -> body | exit
body -> condition
exit -> return
```

对这种形态，WAT 会生成为 structured `block` + `loop`，而不是带 `br_table` 和
`$ik_bb` local 的通用 dispatcher。这会减少 `examples/pricing.ik` 这类 kernel 的
branch dispatch traffic。

`-O3` WASM pipeline 也启用 address CSE，所以重复的 `ptr<Struct>[i].field` load
会复用 indexed base address：

```wat
local.get $items
local.get $i
i32.const 32
i32.mul
i32.add
local.set $addr0

local.get $addr0
i64.load offset=0 align=8
```

fallback 行为：

- 复杂 control flow 仍使用 dispatcher
- short-circuit CFG 继续保持 RHS block 隔离
- 不添加 bounds check
- 不添加 checked WASM arithmetic

### LLVM Hot Path Lowering

LLVM backend 默认生成保守的 alloca/load/store IR，以便统一覆盖 MIR control flow、
call 和 memory operation。在 `-O2` 和 `-O3` 下，它可以对一类非常小的函数绕过
stack slot：

- 单个 basic block
- 没有 `let` local
- 没有 branch、jump 或 loop
- 没有 function call
- 没有 load/store/address operation
- 只包含常量、move、scalar arithmetic、comparison、unary operator 和 return

对这种形态，LLVM backend 会生成直接 SSA-like operation：

```llvm
define i64 @add_i64(i64 %a, i64 %b) {
entry:
  %v0 = add i64 %a, %b
  ret i64 %v0
}
```

更复杂的函数，包括 `examples/pricing.ik`，仍然使用 stack lowering。随后 clang
`-O2`/`-O3` 可以提升许多 stack slot，并优化最终 native code。backend 仍然不会
生成 unsafe `nsw`/`nuw` flag，不添加 bounds check，也不支持 checked LLVM
arithmetic。

`build-llvm` 现在会把选定的 IK optimization level 作为 `-O0`、`-O1`、`-O2` 或
`-O3` 传给 clang。

### Checked C Induction Optimization

checked arithmetic 通常会为每个整数 `+`、`-`、`*` 生成 overflow check，并为
`/` 和 `%` 生成 division check。

在 `-O3` 下，C backend 只有在非常保守的证明成功时，才会把 loop induction
increment 生成为普通 `i + 1`：

- `i` 初始化为 literal `0`
- loop condition 正好是 `i < len`
- `i` 和 `len` 都是 `i32` 或都为 `u32`
- loop 中 `i` 只通过 `i = i + 1` 更新
- loop 中不修改 `len`
- loop body 直接跳回 condition block

任何条件不满足时，checked backend 都保留 `__builtin_add_overflow`。

故意保留的检查：

- `price * qty` 这类业务算术
- discount、tax 和 amount 加减
- division by zero check
- signed min / `-1` check
- unary minus overflow check
- 任何没有被上述精确规则证明安全的 induction update

### CFG Simplification

CFG simplification pass 只改写 basic-block 结构，不移动普通 instruction。

在 `-O1` 下，它只删除 unreachable block。

在 `-O2` 和 `-O3` 下，它还会：

- 将 constant branch 改写成 direct jump
- 将穿过空 jump-only block 的 jump 改写到最终目标
- predecessor 已经改写后删除空 jump-only block

安全边界：

- 不跨 basic block 移动 instruction
- 不复制 instruction
- 不提前执行 `&&` 或 `||` 的 RHS block
- pass 后继续运行 MIR validator

## Debug Flags

CLI 暴露 MIR optimization debug flags：

```sh
ikc emit-mir examples/pricing.ik -O3 --print-pass-pipeline
ikc emit-mir examples/pricing.ik -O3 --print-mir-before-opt
ikc emit-mir examples/pricing.ik -O3 --print-mir-after-opt
```

debug 输出写到 stderr，这样 `emit-mir`、`emit-wat`、`emit-llvm` 等命令的
stdout 仍然可以保持稳定 artifact stream。

## Phase 14 最终行为

Phase 14 中，`-O0` 仍然只运行 validator，并让 generated output 尽量接近
lowered MIR。`-O1` 启用低成本局部清理。`-O2` 启用标准优化管线。`-O3` 启用当前
实现中最激进的管线，并把所选 native optimization level 传给 C 和 LLVM build
command。

当前 pass 仍然故意保持保守。`examples/pricing.ik` 在 checked mode 下仍然保留所有
业务 overflow 和 division check；只有 loop counter increment 会在证明成功后生成为
unchecked arithmetic。WASM 和 LLVM 仍然是 unchecked-only backend，并会拒绝
`--overflow checked`。

## 发布建议

发布 optimization 变更前：

- 运行 `pnpm test`、`pnpm typecheck` 和 `pnpm build`
- 运行 `node --test bench/perf/tests/perf-core.test.mjs`
- 运行 `node bench/perf/run.mjs --quick`
- 如果已有本机 baseline，运行 `node bench/perf/run.mjs --quick --compare`
- 重要 tag 前，在机器时间允许时运行完整 performance pass
- review MIR、C、WAT、LLVM 和 benchmark summary diff，确认没有意外的
  benchmark-specific 行为
