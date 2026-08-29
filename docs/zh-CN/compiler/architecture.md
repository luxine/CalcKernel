# CalcKernel 0.11 编译器架构

[English](../../compiler/architecture.md)

公共行为由语言、CLI、MIR、兼容性与 ABI 文档定义；本文说明实现职责。

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker/contracts
    -> semantic MIR lowering/validation
    -> consumer reachability + mode-specific KIR construction
    -> KIR verifier -> fact-driven KIR optimizer -> KIR verifier
    +-> C source/header
    +-> WAT/WASM
    +-> structural LLVM -> TargetMachine object -> ORC 或进程内 LLD
```

`src/frontend/` 负责 source coordinate、稳定 diagnostic、parser、checked type、
return/unreachable、unsafe block 与 typed closed contract。Contract 是 mathematical
integer 上的 metadata，不是可执行 CK。

`src/ir/mir/` 负责 semantic MIR、deterministic `emit-mir` 与 validation。MIR 保持
source evaluation、possible checked failure 与 print order；它是 mode-neutral 的，不再有
可选 product optimizer。`MirType::Slice`、`MakeSlice`、`SliceIndex`、`Subslice` 保留
descriptor semantics，不加入 backend-specific check。
Structured `break`/`continue` 以 `MirTerminator::Jump` lowering 到对应的最内层 target；
`void` call/return 不创建 synthetic value。

`src/ir/kir/` 构造 consumer/mode-specific KIR。KIR 包含 scalar SSA、block parameter、
region Memory SSA、显式 guard 与 ordered effect。Builder 先按 artifact root 剪除不可达代码，
再拒绝不支持的 runtime capability；library root 是 export，executable root 是 `main`，
inspection root 是两者并集。

`src/optimizer/` 负责 scalar/path、natural-loop、alias/region、Memory SSA、
interprocedural effect analysis，以及 fact/proof arena、pass manager 和独立 evidence
verifier。Fact 区分 proven analysis 与 trusted contract instance；每个 unsafe call 都有独立
scope。没有对当前 CFG/Memory SSA 有效的 closed certificate 或 auditable contract fact，
pass 就不能删除 guard 或输出 backend fact。

`src/backend/` 只消费 verified KIR。C/Native 用显式 guard/status flow 支持四种
overflow/bounds 组合，WASM 仅支持 unchecked。C 可输出 portable restrict/alignment hint；
Native 在 bridge 前运行 fact audit，再把合法 fact 映射到 LLVM attribute/metadata，验证 IR，
最后由 host TargetMachine 输出 object。

`src/backend/llvm/`、`native/bridge/` 负责 Rust/C++ typed ownership；
`native/runtime/` 负责 entry、checked/sanitizer diagnostic 和 print effect；LLD/ORC 均进程内
运行。Public Native C ABI 保持 1，private LLVM bridge 与 contract-aware runtime ABI 为 2。

`src/cli/` 负责参数、dispatch、transactional output、`emit-kir` evidence、contract
sanitizer、isolated run/cache 与 diagnostic。Cache/codegen identity 包含 KIR v1、consumer、
mode、contract、ABI、LLVM、target、CPU feature 与 sanitizer configuration。

Malformed KIR、stale proof/fact、无效 effect order 或 backend fact-audit failure 都会在
artifact commit 前终止；编译器不会回退到未验证的 MIR path。
