# IntKernel / TK LLVM Backend 设计

[English](../LLVM_BACKEND.md)

本文档定义 Phase 13 v1 LLVM backend 设计。在 backend 实现前，它只是设计文档。

## 目标

IntKernel / TK 在 MIR 之后新增 LLVM backend：

```text
.tk source
  -> lexer
  -> parser
  -> AST
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> LLVM IR text backend
  -> .ll
  -> clang / llc
  -> object file or native library
```

Backend 必须消费已验证的 MIR，不能直接从 AST 生成 LLVM IR。

未来 native-library pipeline：

```text
.ll
  -> clang
  -> .so / .dylib / .dll
```

## Phase 13 v1 范围

支持：

- `i32`
- `i64`
- `u32`
- `u64`
- `bool`
- `ptr<T>`
- `struct`
- exported functions
- internal non-exported functions
- scalar arithmetic
- comparisons
- `if` / `else`
- `while`
- function calls
- ptr/index/field load and store
- unchecked arithmetic

暂不支持：

- checked LLVM backend
- optimizer pass pipeline
- LLVM C++ API bindings
- LLVM bitcode writer
- JIT
- debug info
- DWARF
- LTO
- bounds check
- `slice<T>`
- runtime
- allocator
- strings
- IO
- module system

## LLVM IR 生成策略

Phase 13 v1 生成 textual LLVM IR：

```text
MIR -> .ll
```

不嵌入 LLVM library，也不调用 LLVM C++ API。

原因：

- TypeScript 可以稳定生成文本，不需要 native LLVM binding。
- `.ll` 输出可读、可 review。
- snapshot 可以锁定生成 IR 格式和 ABI 形态。
- `clang` 或 `llc` 可以验证语法并编译生成的 IR。

## SSA 策略

LLVM IR 是 SSA，但 MIR v1 不是 SSA。Phase 13 v1 使用 alloca/load/store lowering：

- 每个 parameter、local 和 temporary 都在 entry block 中生成 `alloca`
- 函数入口将 parameter store 到对应 alloca
- 每条 MIR instruction load operand、计算结果、再 store 到 target alloca
- 后续 clang/LLVM optimization 可以通过 mem2reg 提升到寄存器

这不是最优 IR。它刻意简单、正确、稳定且易调试。未来阶段可以增加直接 SSA
lowering 或 MIR-to-SSA transform。

## 类型映射

| IntKernel / TK type | LLVM IR type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `bool` internal | `i1` |
| `ptr<T>` | `ptr` |
| `struct` | named LLVM struct type |

Phase 13 v1 使用 LLVM opaque pointer（`ptr`）。

Signedness 不属于 integer type 本身。Signed/unsigned 差异通过 division、
remainder 和 comparison 指令选择体现。

`bool` ABI 在 v1 中保持保守。内部 boolean value 使用 `i1`。跨语言 bool ABI
不是 Phase 13 v1 的重点；应先覆盖 scalar condition 和 boolean result，在将
exported bool ABI 视为稳定前，需要谨慎记录。

## Struct Types

Struct 降低为 named LLVM struct type：

```llvm
%struct.Item = type { i64, i64, i64, i64 }
```

Field order 遵循源码声明顺序。布局最终由 native compilation 时的 LLVM target data
layout 解释，因此 Phase 13 测试必须在支持的 host 上用 clang 验证关键 ABI 预期。

## Arithmetic 映射

Unchecked arithmetic：

| MIR op | Signed type | Unsigned type |
| --- | --- | --- |
| `+` | `add` | `add` |
| `-` | `sub` | `sub` |
| `*` | `mul` | `mul` |
| `/` | `sdiv` | `udiv` |
| `%` | `srem` | `urem` |

Phase 13 v1 不添加 checked arithmetic guard。如果请求 checked mode，backend 必须报
unsupported-mode error。

## Comparison 映射

Equality：

- `==` -> `icmp eq`
- `!=` -> `icmp ne`

Signed ordering：

- `<` -> `icmp slt`
- `<=` -> `icmp sle`
- `>` -> `icmp sgt`
- `>=` -> `icmp sge`

Unsigned ordering：

- `<` -> `icmp ult`
- `<=` -> `icmp ule`
- `>` -> `icmp ugt`
- `>=` -> `icmp uge`

Comparison result 是 `i1`。

## Control Flow

MIR block 直接映射到 LLVM basic block。

MIR terminator 降低为：

- `return value` -> `ret <type> <value>`
- `jump label` -> `br label %label`
- `branch cond then else` -> `br i1 %cond, label %then, label %else`

Short-circuit behavior 已经表示为 MIR control flow，所以 LLVM backend 必须保持 block
结构，而不是重新计算 logical RHS expression。

## Function Calls

MIR call instruction 降低为 LLVM `call` instruction。

Exported function 示例：

```llvm
define i32 @calc_items(ptr %items, i32 %len, ptr %out) {
  ...
}
```

Internal non-exported function 示例：

```llvm
define internal i64 @add_i64(i64 %a, i64 %b) {
  ...
}
```

Function definition order 应保持稳定。LLVM IR 允许 forward reference，但稳定 module
顺序更适合 snapshot。

## Struct 和 Pointer Access

对于：

```ik
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}
```

LLVM struct type：

```llvm
%struct.Item = type { i64, i64, i64, i64 }
```

`items[i].price` 降低为 GEP + load：

```llvm
%ptr_item = getelementptr %struct.Item, ptr %items, i64 %idx
%ptr_price = getelementptr %struct.Item, ptr %ptr_item, i32 0, i32 0
%price = load i64, ptr %ptr_price
```

`out[i] = value` 降低为 GEP + store：

```llvm
%ptr_out_i = getelementptr i64, ptr %out, i64 %idx
store i64 %value, ptr %ptr_out_i
```

Index expression 由 MIR 在 address lowering 前完成求值。Phase 13 v1 不添加 bounds
check。

## Target Triple

`emit-llvm` 可以支持可选 target triple：

```sh
tkc emit-llvm examples/pricing.tk --out build/pricing.ll --target x86_64-apple-darwin
```

常见 triple：

- `x86_64-apple-darwin`
- `arm64-apple-darwin` 或 `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

如果没有提供 target，Phase 13 v1 可以省略 `target triple` 行，或使用 native target
detection。初版 textual IR backend 可以接受省略。

## CLI 设计

建议命令：

```sh
tkc emit-llvm examples/pricing.tk --out build/pricing.ll
tkc emit-llvm examples/pricing.tk --out build/pricing.ll --target x86_64-apple-darwin

tkc build-llvm examples/pricing.tk --out build/libpricing
```

如果当前 package 仍暴露 `ikc`，可以先在同一 backend 上提供：

```sh
ikc emit-llvm examples/pricing.ik --out build/pricing.ll
ikc build-llvm examples/pricing.ik --out build/libpricing
```

`emit-llvm` 必须是纯文本生成，不依赖 clang 或 LLVM 工具。`build-llvm` 可以调用
clang。

## build-llvm

`build-llvm` 可以通过 clang 编译生成的 `.ll`。

macOS：

```sh
clang -O3 -shared -fPIC build/pricing.ll -o build/libpricing.dylib
```

Linux：

```sh
clang -O3 -shared -fPIC build/pricing.ll -o build/libpricing.so
```

Windows：

```sh
clang -O3 -shared build/pricing.ll -o build/pricing.dll
```

如果 clang 不可用，`build-llvm` 应输出友好错误。`emit-llvm` 必须在没有 clang 时
仍可用。

## Checked Mode

Phase 13 v1 不支持 checked LLVM code generation。

如果用户执行：

```sh
tkc emit-llvm input.tk --overflow checked
```

编译器必须报告：

```text
LLVM backend does not support --overflow checked yet.
```

请求 checked mode 时，backend 不能静默生成 unchecked LLVM IR。

## 测试策略

需要的测试：

- LLVM IR golden snapshots
- clang 可用时的 LLVM syntax smoke test
- clang 可用时将 `.ll` 编译成 executable 或 native library
- scalar e2e
- control-flow e2e
- function-call e2e
- ptr/index/field/store e2e
- pricing e2e
- checked-mode unsupported diagnostic tests
- C backend regression tests
- WASM backend regression tests

生成的 LLVM IR 必须稳定：

- 无绝对路径
- 无时间戳
- 无随机 ID
- 统一 `\n` newline

## 风险

- bool ABI，以及 exported bool result 应使用 `i1`、`i8` 还是 `i32`
- struct layout 和 target data layout 差异
- opaque pointer syntax 与 host clang 版本的兼容性
- Windows linking 和 symbol export 行为
- 本地和 CI 环境中的 LLVM tool availability
- alloca-heavy IR 性能不是最终形态
- 与 C backend ABI 行为保持一致，方便 host-language integration

## 未来工作

- direct SSA lowering
- optional optimizer pipeline
- checked LLVM arithmetic lowering
- target-specific data layout emission
- debug info
- bitcode emission
- object/native-library build hardening
