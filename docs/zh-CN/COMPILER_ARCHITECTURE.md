# IntKernel 编译器架构

[English](../COMPILER_ARCHITECTURE.md)

IntKernel 是用 TypeScript 实现的 source-to-C 和 source-to-WASM 编译器。Phase 13
在 MIR 之后设计未来 source-to-LLVM 路径。

## Pipeline

当前 pipeline：

```text
.ik source
  -> SourceFile
  -> lexer
  -> tokens
  -> parser
  -> AST
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> C backend + header emitter
       -> .c / .h
       -> optional build command
       -> dynamic library
  -> WAT/WASM backend
       -> .wat
       -> .wasm
  -> future LLVM IR backend
       -> .ll
```

Native-library 路径刻意停在可读 C 这一步。本地编译委托给外部 C 编译器，例如
clang。WASM 路径生成 WAT，并可通过 `wabt` assembly 成 `.wasm`。

Phase 12 在 MIR 之后增加已实现的 WASM 路径：

```text
.ik source
  -> lexer
  -> parser
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> MIR WAT backend
  -> .wat
  -> WAT-to-WASM assembly with wabt
  -> .wasm
```

在 WASM backend hardened 期间，C backend 仍是 reference backend。
LLVM backend 是 Phase 13 设计工作，目前还不是默认 pipeline 的一部分。

## 各层职责

### SourceFile

`SourceFile` 持有文件名和源码文本。Lexer、parser、type checker、diagnostics
formatter 和 CLI reporting 都会传递它。

### Lexer

Lexer 将原始 `.ik` 文本转换成 token。每个 token 记录：

- kind
- text
- line
- column
- start offset
- end offset

Lexer 跳过空白和 `//` 单行注释。非法字符会产生 diagnostic，并继续 lexing，
让调用方尽可能报告多个错误。

### Parser

Parser 消费 token 并构建 AST。它是 recursive-descent parser，并用 precedence
parsing 处理表达式。AST 节点携带 source span，方便后续阶段报告有用错误。

Parser diagnostics 使用 source span，并保留给 type checker 和 CLI。

### AST

AST 表示 V0 语言的声明、类型、语句和表达式。它只建模 V0 特性；不支持的语言
特性不会出现在 AST 中。

### Type Checker

Type checker 为 struct、function、parameter 和 local 构建 symbol table。它验证
名称、类型、assignment、function call、control-flow condition、return type、
pointer indexing 和 struct field access。

### CheckedProgram / Typed Program

类型检查成功后，编译器暴露 `CheckedProgram` 这个 typed contract。它保存：

- 原始 AST
- struct 和 function symbol 信息
- parameter、local 和 return type
- expression type 信息
- struct field 信息

MIR lowering 读取这个 contract，而不是直接依赖 checker 内部实现。AST 仍是贴近
源码形状的语法树；`CheckedProgram` 是后续阶段使用的 typed view。

### MIR Lowering

Phase 11 在 type checker 之后引入 Typed MIR。MIR 将 Typed AST 降低成 typed、
three-address、basic-block based 表示。它规范化 control flow 和 lvalue/rvalue，
便于 backend 消费，同时保持源码语义。

MIR lowering 负责：

- 将 `if` / `else` 和 `while` 转换成带 label 的 basic blocks
- 将 `&&` 和 `||` 降低成 control flow，保持 short-circuit 行为
- 将 function call 降低成显式 `call` instruction
- 将 pointer indexing 和 struct field access 降低成 typed places
- 为 snapshot 生成稳定的 temporary 和 block 名称

### MIR Validator

MIR validator 在 C emission 前检查 lowered module。它验证 function name、
block label、terminator、branch target、return type、operand type、function
call signature 和 load/store place。

如果默认 pipeline 产生 invalid MIR，这是 internal compiler error。用户源码错误
应该已经由 lexer、parser 或 type checker 报告。

### MIR C Backend

默认 C source pipeline 会把 checked program 降低到 MIR，验证 MIR，再从 MIR
生成 C。Legacy AST-to-C emitter 仍保留在代码库中，用于对比和 fallback。

MIR v1 不是 SSA，也不做优化。它不添加 bounds check、runtime 或新语言特性。
MIR v1 设计见 [MIR](MIR.md)。

MIR C backend 生成 `.c` 实现文件。它支持两种 overflow mode：

- unchecked mode 生成普通 C expression 和原始 return type
- checked mode 生成 `IK_Status`、checked arithmetic guard、checked function
  call propagation 和 `ik_return` 处理

导出函数声明在 header 中。非导出函数在 C source 中生成为 `static`。

### Header Emitter

Header emitter 由默认 MIR pipeline 共享。它生成 `.h` 文件，包含：

- `#pragma once`
- standard includes
- `IK_API` 和 `IK_BUILD_DLL` 处理
- C++ `extern "C"` guards
- struct typedefs
- exported function declarations

Unchecked header 保持原始 return type。Checked header 包含 `IK_Status`，并在导出
函数签名末尾追加 `ik_return` 指针。

### Build Command

CLI `build` 命令生成 C/header 文件，并用严格参数调用 clang：

```text
-std=c11 -O3 -Wall -Wextra -Werror
```

V0 不捆绑 runtime 或 compiler toolchain。

### WASM Backend

Phase 12 WASM backend 消费已验证的 MIR，并生成稳定 WAT。`emit-wasm` 命令通过
捆绑的 `wabt` npm package 将 WAT assembly 成 `.wasm` binary。目标 ABI 是
`wasm32`：`ptr<T>` 变成 `i32` linear-memory offset，`bool` 使用 `i32`，`i64`
/ `u64` 在 JavaScript 中以 `BigInt` 暴露。

Phase 12 v1 刻意保持窄范围：

- only unchecked arithmetic
- exported linear memory
- deterministic IntKernel struct layout
- scalar expression、control flow、short-circuit、function call 和
  ptr/index/field load/store pattern
- no WASI imports
- no allocator
- no runtime
- no bounds checks

Phase 12 ABI 和使用模型见 [WASM ABI](WASM_ABI.md)。

### 计划中的 LLVM Backend

Phase 13 设计 MIR-to-LLVM textual IR backend。计划中的 backend 消费已验证 MIR，
并生成稳定 `.ll` 文本。v1 刻意不嵌入 LLVM library，也不使用 LLVM C++ API。

计划中的 LLVM path：

```text
.tk / .ik source
  -> AST
  -> CheckedProgram / Typed Program
  -> MIR
  -> MIR validator
  -> LLVM IR text backend
  -> .ll
  -> optional clang / llc build
```

Phase 13 v1 仅支持 unchecked，采用 alloca/load/store lowering，重点覆盖 scalar
operation、control flow、function call 和 ptr/index/field load/store。它不增加
optimizer、checked LLVM lowering、JIT、debug info、runtime、allocator、bounds
check 或 `slice<T>`。

设计见 [LLVM Backend](LLVM_BACKEND.md)。

## Diagnostics 流转

Diagnostics 以数据形式收集并流经 pipeline：

```text
lexer diagnostics
  -> parser diagnostics
  -> type checker diagnostics
  -> CLI formatter
```

每个 diagnostic 包含：

- error code
- severity
- message
- file name
- line
- column
- source span

CLI 会输出文件位置、错误码、消息、源码行和 caret range。

MIR validator failure 会作为 internal compiler error 报告。它们表示类型检查之后
的编译器 bug，而不是用户源码语言 diagnostic。

## 为什么先生成 C

原始 V0 编译器先生成 C，再引入 MIR 和 WASM，原因很务实：

- C 易于检查和 review。
- C ABI 被 Node.js、Python、Java、Rust、Go、C# 等宿主语言广泛支持。
- 现有平台 C 编译器已经负责本地优化和动态库生成。
- 在语言和 ABI 稳定前，这让编译器保持小而清晰。

WASM 在 MIR 成为默认 codegen pipeline 之后从 Phase 12 开始；WASM backend
hardened 前，C backend 继续作为 reference backend。LLVM 从 Phase 13 设计工作开始，
应先通过 textual IR 和 clang/llc validation 证明，再成为 production backend。

## 未来 IR 方向

Phase 11 之前，backend 直接从 checked AST 生成 C。Phase 11 增加 Typed MIR，
长期方向是：

- 用 MIR 提供更简单、规范化的 typed program representation
- 用 MIR 表示 control-flow lowering 和 backend-independent code generation
- 面向 C、WASM 或 LLVM 做 backend-specific lowering

MIR v1 刻意保守：无 SSA、无 optimizer、无 register allocation、无 bounds
check，也不增加新语言特性。
