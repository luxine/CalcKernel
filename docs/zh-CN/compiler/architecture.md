# CalcKernel 0.12 编译器架构

[English](../../compiler/architecture.md)

公共行为由语言、CLI、MIR、兼容性与 ABI 文档定义；本文说明实现职责。

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker/contracts
    -> semantic MIR lowering/validation
    -> consumer reachability + mode-specific KIR v2 construction
    -> target profile -> KIR verifier -> transactional optimizer -> KIR verifier
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

`src/ir/kir/` 构造 consumer、mode、target 与 CPU-specific KIR v2。KIR 包含 scalar/fixed-vector
SSA、block parameter、region Memory SSA、显式 guard、ordered effect、runtime predicate 和
规范化 `KirTargetProfile`。Profile 固定完整 query universe、operation legality/cost、vector lane、
alignment、consumer、target 与 CPU policy；其确定性 digest 进入全部 Native object/cache
identity。Builder 先按 artifact root 剪除不可达代码，再拒绝不支持的 runtime capability；
library root 是 export，executable root 是 `main`，inspection root 是两者并集。

`src/optimizer/` 负责 scalar/path、natural-loop、access/dependence、alias/region、Memory SSA、
SLP 与 interprocedural effect analysis，以及 fact/proof arena、pass manager 和独立 transform
checker。O3 增加有预算上限的 specialization、loop normalization、unroll/SLP 与 Loop SIMD，
包含 scalar epilogue，以及至多一个总 alias predicate 和 scalar fallback。所有 speculative
pass 都先生成完整 candidate state 与 audit delta，针对不可变 verified pre-state 独立检查，
然后同时原子提交或同时回滚。Fact 区分 proven analysis 与 trusted contract instance；每个
unsafe call 都有独立 scope。没有对当前 CFG/Memory SSA 有效的 closed certificate 或
auditable contract fact，pass 就不能删除 guard、复制 region 或输出 backend fact。

`src/backend/` 只消费 verified KIR。C/Native 用显式 guard/status flow 支持四种
overflow/bounds 组合，WASM 仅支持 unchecked。0.12 的 C 与 WebAssembly profile 明确禁用
Vector KIR，因此二者继续消费 verified scalar KIR，同时保留有收益的 scalar specialization
与 cleanup。C 可输出 portable restrict/alignment hint；Native 结构化 lowering 已检查的
Vector KIR，并在 bridge 前运行 fact audit，再把合法 scalar fact 映射到 LLVM
attribute/metadata，验证 IR，最后由 host TargetMachine 输出 object。

`src/backend/llvm/`、`native/bridge/` 负责 Rust/C++ typed ownership；
`native/runtime/` 负责 entry、checked/sanitizer diagnostic 和 print effect；LLD/ORC 均进程内
运行。Public Native C ABI 保持 1，private LLVM bridge ABI 为 3，contract-aware runtime ABI
保持 2。

`src/cli/` 负责参数、dispatch、transactional output、`emit-kir` evidence、contract
sanitizer、isolated run/cache 与 diagnostic。Cache/codegen identity 包含 KIR v2、consumer、
mode、contract、ABI、LLVM、target-profile digest、CPU feature、optimizer proof/cost schema、
budget 与 sanitizer configuration。Native entry 使用 `CKCOBJ02` manifest schema 3。

Malformed KIR、stale proof/fact、无效 effect order 或 backend fact-audit failure 都会在
artifact commit 前终止；编译器不会回退到未验证的 MIR path。
