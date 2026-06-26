# CK / CalcKernel LLVM Backend

[English](../LLVM_BACKEND.md)

本文档定义 Phase 13 v1 LLVM backend 的行为、ABI 假设、外部工具依赖、限制和未来工作。

## 目标

CK / CalcKernel 在 MIR 之后新增 LLVM backend：

```text
.ck source
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

Native-library pipeline：

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
- `f64` scalar arithmetic 和 comparisons
- `if` / `else`
- `while`
- function calls
- ptr/index/field load and store
- `ptr<f64>` 和包含 `f64` 的 struct field
- unchecked arithmetic

暂不支持：

- checked LLVM backend
- LLVM-specific optimizer pass pipeline
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

这不是最优 IR。它刻意简单、正确、稳定且易调试。

Phase 14.14 在 `-O2` 和 `-O3` 下为简单 scalar straight-line function 增加一个
小型 SSA-like fast path。这个 fast path 仅限单个 basic block、没有 local、没有
call、没有 control flow、没有 memory operation 的函数。更复杂的函数，包括
`examples/pricing.ck`，仍然使用 stack lowering，并依赖 clang `-O2`/`-O3` 做
promotion 和 native optimization。未来阶段可以扩大 direct SSA lowering，或增加
MIR-to-SSA transform。

## 类型映射

| CK / CalcKernel type | LLVM IR type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `f64` | `double` |
| `bool` internal 和 exported scalar | `i1` |
| `ptr<T>` | `ptr` |
| `struct` | named LLVM struct type |

Phase 13 v1 使用 LLVM opaque pointer（`ptr`）。
Phase 16.8 支持把 `f64` 映射为 LLVM `double`，包括 scalar parameter/return、
`ptr<f64>` memory access、struct field `double`、arithmetic、unary negation 和
comparison。LLVM f64 codegen 不添加 fast-math flags，也不承诺所有 backend 的结果
bit-identical。
Phase 20 支持 exact explicit `i32_to_f64` 和 `u32_to_f64` cast，分别使用
`sitofp` 和 `uitofp`。

Signedness 不属于 integer type 本身。Signed/unsigned 差异通过 division、
remainder 和 comparison 指令选择体现。

### Bool ABI

Phase 13.7 保持策略 A：

- internal bool value 使用 `i1`
- condition 使用 `i1`
- bool local 和 temporary 使用 `i1`
- exported bool parameter 和 return value 使用 `i1`

在当前 macOS clang target 上，编译等价 C：

```c
#include <stdbool.h>
bool less_i64(long long a, long long b) { return a < b; }
bool not_bool(bool a) { return !a; }
int choose_bool(bool a, int x, int y) { return a ? x : y; }
```

会生成使用 `i1` 表示 bool parameter 和 return 的 LLVM IR，clang 会在部分
signature 上附加 `zeroext` 等 ABI attribute。CalcKernel LLVM v1 目前生成 plain
`i1`；Phase 13.7 clang e2e 已验证当前 target 上，C harness 可以用 `bool` 调用
exported LLVM functions，覆盖 bool return、bool parameter、bool local，以及
基于 bool parameter 的 `if`。

跨平台 bool ABI 仍是风险。如果 Windows 或其他 target 需要 `zeroext` 等 attribute
才能稳定 interop，后续 hardening 阶段应增加 target-aware function parameter 和
return attribute，而不是改变语言层面的 public type。

## Struct Types

Struct 降低为 named LLVM struct type：

```llvm
%struct.Item = type { i64, i64, i64, i64 }
```

`f64` field 降低为 `double`：

```llvm
%struct.WithF64 = type { i32, double }
```

Field order 遵循源码声明顺序。布局最终由 native compilation 时的 LLVM target data
layout 解释，因此 Phase 13 测试必须在支持的 host 上用 clang 验证关键 ABI 预期。

## Arithmetic 映射

Unchecked arithmetic：

| MIR op | Signed integer | Unsigned integer | `f64` |
| --- | --- | --- | --- |
| `+` | `add` | `add` | `fadd` |
| `-` | `sub` | `sub` | `fsub` |
| `*` | `mul` | `mul` | `fmul` |
| `/` | `sdiv` | `udiv` | `fdiv` |
| `%` | `srem` | `urem` | unsupported |

Unary `-f64` 降低为 `fneg double`。Backend 不能把 f64 negation 降低为 integer
zero subtraction。

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

F64 comparison：

- `==` -> `fcmp oeq`
- `!=` -> `fcmp une`
- `<` -> `fcmp olt`
- `<=` -> `fcmp ole`
- `>` -> `fcmp ogt`
- `>=` -> `fcmp oge`

Comparison result 是 `i1`。

f64 语义锁定为 strict LLVM IR：

- 生成不带 fast-math flag 的 `fadd`、`fsub`、`fmul`、`fdiv`、`fneg` 和 `fcmp`
- `i32_to_f64` 生成 `sitofp i32 ... to double`
- `u32_to_f64` 生成 `uitofp i32 ... to double`
- 不使用 reassociation、reciprocal、no-NaN、no-infinity、signed-zero-ignore
  或其他 fast-float 假设
- NaN、infinity 和 `-0.0` 遵循编译目标的普通 IEEE-like 行为
- 不承诺 NaN payload 稳定，也不承诺和 C、WASM 或 JavaScript host bit-identical

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

```ck
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

Index expression 由 MIR 在 address lowering 前完成求值。Phase 13 v1 在把 index
用于 LLVM `i64` GEP index 前，会对 `i32` 使用 `sext`，对 `u32` 使用 `zext`。
Phase 13 v1 不添加 bounds check，也不检查 pointer 是否为 null。

Phase 13 的 memory e2e 测试覆盖 integer pointer 和 integer struct field。
Bool scalar ABI 单独覆盖，但 bool field 或 bool pointer 还不作为跨语言稳定的
LLVM memory ABI。

`ptr<f64>` 在 function boundary 使用 opaque `ptr`，indexed access 降低为
`getelementptr double`。F64 load/store 使用 `load double` 和 `store double`。
声明为 `f64` 的 struct field 在 LLVM named struct 中使用 `double`，field access
沿用 integer field 的 struct GEP 模式。

## Target Triple

`emit-llvm` 可以支持可选 target triple：

```sh
ckc emit-llvm examples/pricing.ck --out build/pricing.ll --target x86_64-apple-darwin
```

常见 triple：

- `x86_64-apple-darwin`
- `arm64-apple-darwin` 或 `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

如果没有提供 target，Phase 13 v1 可以省略 `target triple` 行，或使用 native target
detection。初版 textual IR backend 可以接受省略。

`build-llvm --target <triple>` 会把 target triple 写入生成的 `.ll`。Phase 13.14
不会把 `--target` 继续传给 clang；target-aware clang argument handling 是后续
hardening 工作。

## CLI

`emit-llvm` 写出 textual LLVM IR，不依赖 clang：

```sh
ckc emit-llvm examples/pricing.ck --out build/pricing.ll
ckc emit-llvm examples/pricing.ck --out build/pricing.ll --target x86_64-apple-darwin
```

如果省略 `--out`，`emit-llvm` 会把 `.ll` 文本输出到 stdout。

`build-llvm` 会生成 LLVM IR，写入临时 `.ll`，并调用 clang：

```sh
ckc build-llvm examples/pricing.ck --out build/libpricing
ckc build-llvm examples/pricing.ck --kind object --out build/pricing.o
```

如果当前 package 仍暴露 `ckc`，可以先在同一 backend 上提供：

```sh
ckc emit-llvm examples/pricing.ck --out build/pricing.ll
ckc build-llvm examples/pricing.ck --out build/libpricing
```

`emit-llvm` 是纯文本生成，不依赖 clang 或 LLVM 工具。`build-llvm` 要求 `PATH`
中存在 clang。

## build-llvm

`build-llvm` 可以通过 clang 编译生成的 `.ll`。

Dynamic library output 是默认目标：

选定的 CK optimization level 会以 `-O0`、`-O1`、`-O2` 或 `-O3` 传给 clang。
下面的例子展示 `--opt-level 3`。

macOS：

```sh
ckc build-llvm examples/pricing.ck --out build/libpricing --opt-level 3
clang -O3 -shared -fPIC build/libpricing.ll -o build/libpricing.dylib
```

Linux：

```sh
ckc build-llvm examples/pricing.ck --out build/libpricing --opt-level 3
clang -O3 -shared -fPIC build/libpricing.ll -o build/libpricing.so
```

Windows：

```sh
ckc build-llvm examples/pricing.ck --out build/pricing --opt-level 3
clang -O3 -shared build/pricing.ll -o build/pricing.dll
```

如果 `--out` 没有 dynamic-library 扩展名，`build-llvm` 会按平台补扩展：
macOS 使用 `.dylib`，Linux 使用 `.so`，Windows 使用 `.dll`。如果用户传入以
`.so`、`.dylib` 或 `.dll` 结尾的完整文件名，则尊重该文件名。

Object output 可用于用户自己管理的链接流程：

```sh
ckc build-llvm examples/pricing.ck --kind object --out build/pricing.o --opt-level 3
clang -O3 -c build/pricing.ll -o build/pricing.o
```

macOS 和 Linux 通常使用 `.o`。Windows 上，
`build-llvm --kind object --out build/pricing` 使用 `.obj`；如果用户显式传入
`.o` 文件名，则允许 clang 生成该文件。Phase 13.15 不实现 static library
output；调用方可以在 CalcKernel 外部自行使用 linker、`ar` 或 `llvm-ar`。

如果 clang 不可用，`build-llvm` 会输出友好错误，并提示仍可使用 `emit-llvm`
在没有 clang 的情况下生成 LLVM IR。`emit-llvm` 必须在没有 clang 时仍可用。

## Checked Mode

Phase 13 v1 不支持 checked LLVM code generation。

LLVM backend 会拒绝两个 LLVM 入口的 checked mode：

```sh
ckc emit-llvm input.ck --overflow checked
ckc build-llvm input.ck --overflow checked
```

编译器必须报告：

```text
LLVM backend does not support --overflow checked yet.
Use --overflow unchecked, or use the C backend for checked arithmetic.
```

请求 checked mode 时，backend 不能静默生成 unchecked LLVM IR。需要 checked
arithmetic 时应使用 C backend（`emit-c` 或 `build`）。

## 测试策略

需要的测试：

- `emit-llvm` CLI tests
- `build-llvm` clang command tests
- LLVM IR golden snapshots
- clang 可用时的 LLVM syntax smoke test
- clang 可用时将 `.ll` 编译成 executable 或 native library
- scalar e2e
- control-flow e2e
- function-call e2e
- ptr/index/field/store e2e
- f64 scalar、pointer 和 struct-field e2e
- f64 LLVM IR no-fast-math regression
- pricing e2e
- checked-mode unsupported diagnostic tests
- object output tests
- C backend regression tests
- WASM backend regression tests
- C/WASM/LLVM backend behavior comparison tests

生成的 LLVM IR 必须稳定：

- 无绝对路径
- 无时间戳
- 无随机 ID
- 统一 `\n` newline

## 风险

- bool ABI attribute（例如 `zeroext`）可能需要 target-aware hardening
- struct layout 和 target data layout 差异
- opaque pointer syntax 与 host clang 版本的兼容性
- Windows linking 和 symbol export 行为
- 本地和 CI 环境中的 LLVM tool availability
- alloca-heavy IR 性能不是最终形态
- 与 C backend ABI 行为保持一致，方便 host-language integration
- cross-backend f64 precision 和 NaN 行为在 target-specific edge case 上可能不同

## 当前限制

- LLVM backend 只支持 unchecked arithmetic。
- `emit-llvm --overflow checked` 和 `build-llvm --overflow checked` 会以文档化的
  unsupported-mode diagnostic 失败。
- 通用 control-flow 和 memory function 仍使用 alloca/load/store lowering。
- 只有简单 scalar straight-line function 在 `-O2` 和 `-O3` 下使用 SSA-like
  lowering。
- f64 codegen 默认 strict，不输出 fast-math flags。
- f64 `%`、f32、implicit int/float conversion、`i64_to_f64`、`u64_to_f64`、
  f64-to-int cast、float checked overflow、SIMD 和 JIT 仍未支持。
- CalcKernel 不运行 LLVM-specific optimizer pass pipeline；backend input 仍会经过
  共享 MIR pass manager。
- `build-llvm` 依赖外部 clang；CalcKernel 不捆绑 clang、`llc`、LLVM library 或
  custom linker。
- CalcKernel 暂不生成 static library。
- 不生成 debug info、DWARF、LTO 或 bitcode。
- 不提供 runtime、allocator、JIT、strings、IO、modules、bounds checks 或
  `slice<T>`。
- Raw pointer validity 和 buffer size 仍是调用方责任。
- 跨平台 bool ABI attribute 和 target-specific data layout 仍需要后续 hardening，
  才能提供更广泛的 FFI 保证。

## 未来工作

- checked LLVM arithmetic lowering
- 更广泛的 direct SSA LLVM lowering
- optional optimizer pass pipeline
- target-specific data layout emission
- object/static library output improvements
- target-aware clang argument handling
- debug info 和 DWARF
- bitcode emission
- 如果未来产品场景需要，再考虑 JIT
- 语言具备携带长度的 pointer type 后，再支持 `slice<T>` / bounds check
