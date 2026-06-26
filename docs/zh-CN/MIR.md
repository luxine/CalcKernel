# CalcKernel MIR

[English](../MIR.md)

MIR 是 CalcKernel 的 middle-level intermediate representation。它位于 type
checking 之后、backend-specific code generation 之前。它的职责是把 Typed AST
降低成 typed、规范化的结构，让 C、WASM、LLVM 和未来 checked code generation
更容易消费。

## 目标

默认 C codegen pipeline 是：

```text
.ck source
  -> lexer
  -> parser
  -> AST
  -> type checker
  -> Typed Program
  -> MIR
  -> C backend
  -> .c / .h
```

各 backend 复用同一套 MIR，而不是各自重新解释 AST：

```text
MIR -> selected optimization level 下的 MIR pass manager -> C
MIR -> selected optimization level 下的 MIR pass manager -> WASM
MIR -> selected optimization level 下的 MIR pass manager -> LLVM
```

MIR v1 必须保持当前源码语言语义。它是架构层，不是语言功能。

从 Phase 11.15 开始，`ckc emit-c` 和 `ckc build` 默认使用这条 MIR pipeline，
同时覆盖 unchecked 和 checked C generation。Legacy AST-to-C backend 仍作为内部
回归对比和 fallback 保留。

## 非目标

MIR v1 明确不添加：

- SSA
- register allocation
- bounds check
- runtime
- new language features

Phase 14 增加了保守 MIR pass manager，并提供文档化的 O0/O1/O2/O3 pipeline。这些
pass 运行在 MIR v1 上，保持所选 overflow mode 和 backend ABI，详见
[Optimization](OPTIMIZATION.md)。

MIR v1 应该让后续工作更容易，但不能改变 V0 behavior、unchecked ABI、checked
ABI 或 diagnostics 语义。

## MIR v1 范围

MIR v1 覆盖当前 V0 语言面：

- scalar integer 和 boolean expression
- strict `f64` expression
- exact explicit `i32_to_f64` / `u32_to_f64` cast
- `let`、assignment 和 `return`
- `if` / `else`
- `while`
- function call
- short-circuit logical operator
- pointer indexing
- struct field access
- 通过 typed place 进行 load 和 store

它是规范化层，不引入新语法、新类型、新 runtime behavior 或新 safety check。

## 核心结构

MIR v1 是 typed、three-address、basic-block based。

```text
MirModule
  structs: MirStruct[]
  functions: MirFunction[]

MirFunction
  name: string
  exported: bool
  params: MirParam[]
  returnType: MirType
  locals: MirLocal[]
  blocks: MirBlock[]

MirBlock
  label: string
  instructions: MirInstruction[]
  terminator: MirTerminator
```

### Instructions

MIR v1 instruction 用显式 result value 或显式 place 描述简单操作：

- `ConstInt`
- `ConstFloat`
- `ConstBool`
- `Move`
- `Binary`
- `Unary`
- `Compare`
- `Cast`
- `Load`
- `Store`
- `Call`

Arithmetic 和 comparison operation 应该是 three-address operation。例如：

```text
%t0: i64 = add a, b
%t1: bool = lt %t0, c
%t2: f64 = cast i32_to_f64 i
```

当前 MIR cast operation 只有 `i32_to_f64` 和 `u32_to_f64`。MIR validator 会拒绝
其他 cast kind、错误 input type 和错误 result type。

### Terminators

每个 block 以一个 terminator 结束：

- `Return`
- `Jump`
- `Branch`

Terminator 拥有 control flow。普通 instruction 不应该隐式跳转到其他 block。

### Places

`MirPlace` 表示可读或可写的 storage location：

- `Local`
- `Param`
- `Index`
- `Field`

示例：

```text
local sum
param items
index items, i
field (index items, i), price
```

## Types

每个 MIR value 必须有已解析的 CalcKernel 类型：

- `i32`
- `i64`
- `u32`
- `u64`
- `f64`
- `bool`
- `ptr<T>`
- `struct`

MIR 不应包含 `NamedTypeNode` 这类 parser-only type 或未解析 type node。Type
checker 必须在 lowering 前解析名称。

Integer literal 应 materialize 成 type checker 选择的具体类型。MIR 不应需要 AST
专用的 `integerLiteral` pseudo-type。

## Control Flow

MIR v1 将结构化 control flow 降低成 basic block。

`if / else` 降低为 condition value 和 `Branch`：

```text
bb0:
  %cond: bool = lt a, b
  branch %cond, bb_then, bb_else

bb_then:
  return a

bb_else:
  return b
```

`while` 降低成 condition block、body block 和 exit block：

```text
bb0:
  jump bb_cond

bb_cond:
  %cond: bool = lt i, len
  branch %cond, bb_body, bb_exit

bb_body:
  ...
  jump bb_cond

bb_exit:
  return 0
```

Builder 应保持源码级求值顺序。

## Function Call Lowering

Function call 降低成带 result value 的显式 `Call` instruction：

```text
%t0: i64 = call add_i64(a, b)
```

Nested call 按源码参数顺序从内到外降低：

```ck
double_i64(add_i64(a, b))
```

概念上变成：

```text
%t0: i64 = call add_i64(a, b)
%t1: i64 = call double_i64(%t0)
```

Argument expression 在 call instruction 生成前按从左到右求值。即使 call 只作为
statement 使用，也会产生临时 result，让调用在 MIR 中显式存在；MIR v1 不删除
未使用调用。

MIR validator 检查 callee 是否存在、参数数量和类型是否匹配，以及 call result
type 是否等于 callee return type。

## Short-Circuit Logic

`&&` 和 `||` 不能降低成普通 `Binary` instruction。它们必须降低成 control flow，
保证 right-hand side 只在需要时求值。

对 `a && b`：

- 先 evaluate `a`
- 如果 `a` 为 false，结果为 false
- 否则 evaluate `b`

对 `a || b`：

- 先 evaluate `a`
- 如果 `a` 为 true，结果为 true
- 否则 evaluate `b`

这对 checked mode 很重要，因为被跳过的 right-hand side 不能触发 overflow 或
division-by-zero 检查。

## Lvalues 和 Rvalues

MIR v1 区分 place 和 value。

读取：

```ck
items[i].price
```

表示为从 field place 进行 `Load`：

```text
%idx: i32 = move i
%tmp: i64 = load field(index(items, %idx), price)
```

写入：

```ck
out[i] = value;
```

表示为对 index place 的 `Store`：

```text
%idx: i32 = move i
store index(out, %idx), value
```

Index expression 可以包含 arithmetic。Index expression 本身先降低成 MIR value，
然后再形成最终 `Index` place。

## Pointer、Index 和 Field Lowering

Pointer、index 和 field access 通过 place 表示。

读取：

```ck
items[i].price
```

先 evaluate index expression，再从 field place load：

```text
%idx: i32 = move i
%value: i64 = load field(index(items, %idx), price)
```

写入：

```ck
out[i] = value;
```

降低成 store：

```text
%idx: i32 = move i
store index(out, %idx), value
```

复合 index：

```ck
items[i + 1].price
```

会先降低 arithmetic，再形成 place：

```text
%idx: i32 = add i, 1
%value: i64 = load field(index(items, %idx), price)
```

MIR v1 仍然不添加 bounds check、pointer validity check 或 buffer length check。
这些需要未来的语言级 metadata，例如 `slice<T>`。

## Checked Mode 关系

MIR v1 表达普通 CalcKernel arithmetic 语义。它不直接编码 overflow check。

Backend 根据请求的 overflow mode 决定如何生成 arithmetic：

- `unchecked`：生成普通 C operation
- `checked`：生成 overflow guard、division check、`CK_Status` propagation 和
  checked return-pointer handling

这让 MIR 不绑定某个 backend 的 checked C 实现，同时仍让 checked lowering 更一致。

Short-circuit operator 已经表示为 MIR control flow，因此 checked C emission 不需要
单独特殊处理 logical operator，也不会误提前 evaluate right-hand side。Function call
是显式 MIR `Call` instruction，因此 checked C emission 可以在每个 call site 插入
`CK_Status` propagation。

Checked mode 中的 pointer/index/field access 复用相同 MIR place。Index expression
中的 arithmetic 会被检查，因为它在最终 place 使用前已经表示为普通 MIR arithmetic。
MIR 不意味着 bounds check。

## 为什么 MIR v1 不是 SSA

MIR v1 刻意避免 SSA，让项目可以在多个 backend 之间共享 code generation，而不用
同时引入 phi node 和 dominance rule。当前语言有 mutable local 和结构化 loop；用
basic block、显式 move 和 store 表示它们，已经足够生成可读 C、降低到 WASM/LLVM，
并做 backend parity 测试。

这让 MIR snapshot 保持易读。Phase 14 optimization 仍然是保守 MIR-to-MIR pass，
不是 SSA rewrite。

## 未来 SSA 和 Optimizer

未来阶段可以引入 SSA-based IR，或在 MIR v1 之上做更广泛的 optimizer 工作。候选
方向包括：

- range analysis，用于未来 checked 或 bounds-safe 功能
- lower 到 backend-specific SSA，供 LLVM 或 WASM 使用
- 更广泛的 f64 optimization 需要先明确 strict-safe floating point optimization rules
- 默认 build 之外的可选 CPU-native 或 LTO 实验

这些 pass 应该只在 MIR v1 作为默认 C pipeline 稳定至少一个 release 后添加，并且
需要先明确它们对 diagnostics、snapshot 和 generated ABI 的影响。

## Text Format

MIR 应有稳定的文本格式用于 snapshot。格式应避免绝对路径、时间戳、随机 ID 或
平台相关换行。

示例：

```text
fn add_i64(a: i64, b: i64) -> i64 {
bb0:
  %t0: i64 = add a, b
  return %t0
}
```

带 branch 的示例：

```text
export fn max_i32(a: i32, b: i32) -> i32 {
bb0:
  %t0: bool = gt a, b
  branch %t0, bb1, bb2

bb1:
  return a

bb2:
  return b
}
```

Printer 应生成稳定 temporary name，例如 `%t0`、`%t1`，以及 `bb0`、`bb1`、
`bb2` 这类 block label。

## Phase 11 迁移策略

Phase 11 是增量迁移。MIR code generation 通过 snapshot 和 e2e 测试验证期间，
AST backend 保留为 legacy/internal path。

1. 添加 MIR types、printer 和 validator。
2. 添加 Typed AST 到 MIR 的 lowering。
3. 添加 MIR 到 C unchecked backend。
4. 添加 MIR 到 C checked backend。
5. 在 generated C snapshot 和 e2e 测试证明输出符合预期后，将默认 C backend
   切到 MIR。已在 Phase 11.15 完成。
6. 只有在 MIR backend 覆盖当前完整 V0 surface，并作为默认 backend 经历一个
   release cycle 后，才考虑保留或移除旧 AST backend。

每一步都应保留现有 CLI 行为、generated ABI、diagnostics 和测试预期，除非有明确
review 的 snapshot 更新。
