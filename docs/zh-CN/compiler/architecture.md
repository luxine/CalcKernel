# CalcKernel Compiler 架构

[English](../../compiler/architecture.md)

本文解释原生 Rust compiler 的组织；公共行为由 reference/ABI 文档定义，不由
module 位置定义。

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker
    -> MIR lowering/validation -> MIR optimizer
    -> C | WAT/WASM | LLVM -> optional clang build
```

`src/frontend/` 负责 source coordinate、稳定 diagnostic、token、AST、parser、
scope、type checking、definite-return/unreachable analysis 与 typed metadata。
Parser 接受 syntactic type form；type checker 解析名称并执行严格 value/control-flow
规则。

`src/ir/` 负责 MIR model、lowering、deterministic printing 与 validation。
Structured `if`、`while`、`break`、`continue` 变为 branch 与
`MirTerminator::Jump`；void call/return 保持 targetless/valueless；
source return-only type 为 `void`；
`MirType::Slice`、`MakeSlice`、`SliceIndex`、`Subslice` 保留 typed descriptor
operation，但不增加 backend-specific check。

`src/optimizer/` 负责 context、analysis、pass、pipeline、逐 pass validation 与
debug record。Pass 不可改变 diagnostic、mode、evaluation/error order 或 ABI。

`src/backend/` 负责 C、WASM、LLVM plan/emission。每个 backend 消费 validated
MIR 并显式实现自身 ABI；只有 C 实现 checked status mode。相同 source、flag、
target 与 compiler version 必须得到确定输出。

`src/cli/` 负责 argument、dispatch、atomic output、toolchain 与 stdout/stderr；
`src/bin/ckc.rs` 只是薄 process entry；`src/lib.rs` 通过明确 re-export 保持公共
Rust surface。CK 不提供 runtime allocator，所有 generated memory 都由 caller 拥有。
