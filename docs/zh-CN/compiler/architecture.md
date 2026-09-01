# CalcKernel 0.13 编译器架构

[English](../../compiler/architecture.md)

公共行为由语言、CLI、MIR、兼容性与 ABI 文档定义；本文说明实现职责。

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker/contracts
    -> semantic MIR lowering/validation
    -> consumer reachability + mode-specific KIR v3 construction
    -> optional CK workload profile (non-proof) + target profile
    -> KIR verifier -> transactional optimizer -> KIR verifier
    -> optional same pre-state CPU variants -> baseline-safe dispatcher
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

`src/ir/kir/` 构造 consumer、mode、target 与 CPU-specific KIR v3。KIR 包含 scalar/fixed-vector
SSA、block parameter、region Memory SSA、显式 guard、ordered effect、runtime predicate 和
规范化 `KirTargetProfile`。Profile 固定完整 query universe、operation legality/cost、vector lane、
alignment、consumer、target 与 CPU policy；其确定性 digest 进入全部 Native object/cache
identity。Builder 先按 artifact root 剪除不可达代码，再拒绝不支持的 runtime capability；
library root 是 export，executable root 是 `main`，inspection root 是两者并集。

`src/profile/` 负责 CK workload profile，与静态 `KirTargetProfile` 分离。Canonical
`CKPART01` shard merge 为一个已验证 `CKPROF01` terminal profile；使用前完整
compiler/source/KIR/site/target/mode identity 必须匹配。Profile 是 immutable non-proof
evidence，只记录 frequency/work，不能建立 range、alias、alignment、effect、bounds、
dominance 等安全事实。CFG 变化后的 profile mapping 必须有由独立 checker 验证的闭合
transfer record；歧义时回退 ordinary optimization。

`src/optimizer/` 负责 scalar/path、natural-loop、access/dependence、alias/region、Memory SSA、
SLP 与 interprocedural effect analysis，以及 fact/proof arena、pass manager 和独立 transform
checker。O3 增加有预算上限的 specialization、loop normalization、unroll/SLP 与 Loop SIMD，
包含 scalar epilogue，以及至多一个总 alias predicate 和 scalar fallback。所有 speculative
pass 都先生成完整 candidate state 与 audit delta，针对不可变 verified pre-state 独立检查，
然后同时原子提交或同时回滚。Fact 区分 proven analysis 与 trusted contract instance；每个
unsafe call 都有独立 scope。没有对当前 CFG/Memory SSA 有效的 closed certificate 或
auditable contract fact，pass 就不能删除 guard、复制 region 或输出 backend fact。
O2 先冻结 ordinary machine pipeline，只允许 `CkLateProfileLayout` 使用 profile；accepted
delta 仅限 block/trace order 与闭合 target repair allowlist，不能加入 LLVM profile metadata。
O3 的 profile-guided transform 都是从同一 immutable pre-state 提出的 transaction；proposer
不能自我批准，被拒工作仍消耗 budget，每个 candidate 要么提交完整 verified state，要么保持
pre-state 不变。multiversion planning 让 baseline 与全部 enhanced variant 从 same pre-state
开始，分别验证，并禁止 cross-variant LTO。

`src/backend/` 只消费 verified KIR。C/Native 用显式 guard/status flow 支持四种
overflow/bounds 组合，WASM 仅支持 unchecked。0.13 的 C 与 WebAssembly profile 明确禁用
Vector KIR，因此二者继续消费 verified scalar KIR，同时保留有收益的 scalar specialization
与 cleanup。C 可输出 portable restrict/alignment hint；Native 结构化 lowering 已检查的
Vector KIR，并在 bridge 前运行 fact audit，再把合法 scalar fact 映射到 LLVM
attribute/metadata，验证 IR，最后由 host TargetMachine 输出 object。

`src/backend/llvm/`、`native/bridge/` 负责 Rust/C++ typed ownership；
`native/runtime/` 负责 entry、checked/sanitizer diagnostic 和 print effect；LLD/ORC 均进程内
运行。Public Native C ABI 保持 1，private LLVM bridge ABI 为 4，contract-aware runtime ABI
保持 2。

`native/profile_runtime/` 只存在于 generation artifact，通过 directory-anchored transaction
发布 completed shard。Library user 在 quiescence 后调用 full-identity
`ck_profile_flush_*` control symbol。`native/dispatch_runtime/` 负责 baseline-safe detector 与
process-local acquire-release publication；public ABI thunk 保持稳定，baseline、variant 与
runtime implementation 是隐藏的 named-object member。

`src/cli/` 负责参数、dispatch、transactional output、`emit-kir` evidence、contract
sanitizer、isolated run/cache 与 diagnostic。Cache/codegen identity 包含 KIR v3、consumer、
mode、contract、ABI、LLVM、target-profile digest、CPU feature、optimizer proof/cost schema、
budget 与 sanitizer configuration。Native entry 使用 `CKCOBJ03` key schema 4 和 manifest
schema 4。Multiversion cache hit 要求闭合有序 bundle、每个 named object、target set、
dispatcher/runtime、profile 与 physical artifact identity 全部匹配；generation 绕过 cache。

Malformed KIR、stale proof/fact、无效 effect order 或 backend fact-audit failure 都会在
artifact commit 前终止；编译器不会回退到未验证的 MIR path。
