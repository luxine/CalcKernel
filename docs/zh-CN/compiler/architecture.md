# CalcKernel Compiler 架构

[English](../../compiler/architecture.md)

本文解释原生 Rust compiler 的组织；公共行为由 reference/ABI 文档定义，不由
module 位置定义。

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker
    -> MIR lowering/validation -> MIR optimizer
    +-> C source/header
    +-> WAT/WASM
    +-> structural LLVM -> TargetMachine object -> ORC 或 in-process LLD
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

`src/backend/` 负责 C、WASM、Native ABI classification 与 LLVM lowering。C 和
Native 实现 checked status mode，WASM 仅支持 unchecked。Native LLVM 采用 structural
construction，在 optimization 前后 verify，并由 host TargetMachine 生成 object bytes。

`src/backend/llvm/` 与 `native/bridge/` 提供 Rust/C++ boundary 的 typed ownership；
`native/runtime/` 负责 entry、status diagnostic 与 print effect，LLD/ORC 在进程内使用。
`src/cli/` 负责 argument、dispatch、transactional output、isolated run/cache 与 stdout/stderr；
`src/bin/ckc.rs` 是薄 process entry。CK 不提供 runtime allocator，CK-visible memory 由
caller 拥有。
