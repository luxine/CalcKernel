# 阶段 15 任务：Predicated-Update 物化、独立 Checker 与 LLVM

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

把阶段 14 的合法候选物化为固定宽度 vector compare、
`select(candidate, old)` 和一次 unmasked vector store，并由不调用 proposer 的
checker 重建验证；保留 runtime guard、scalar epilogue、strict-f64 和 checked
first-error 语义，最终在 LLVM IR 与真实动态库中可观察。

## 仓库落点与接口

- 修改 `src/optimizer/kir_passes/vectorize.rs`，为
  `VectorScheduleItem` 增加专用 `PredicatedUpdate` 项；不得让现有
  `MergeMemory` 的 equal-arm 要求宽松接受任意 memory diamond。
- 专用物化流程从同 place 产生 vector old load，向量化 candidate，生成
  `VectorCompare(Lt)`、`VectorSelect { when_true: candidate,
  when_false: old }`，再生成恰好一个 `VectorStore`。store 不带 mask；其 memory
  input 是 old load 所见版本，output 替代 merge 的新版本。
- `VectorizationPlan` 继续复用现有 operation/memory-group/predicate/epilogue
  wire-free 内存结构；compare/select/load/store mapping 必须完整进入 plan 和
  post-state digest。
- 修改 `src/optimizer/vectorize_check.rs`，独立从 pre-state 重建
  `VectorPredicatedUpdate`，检查 exact place、dominance、Memory SSA、alias/effect、
  branch polarity、compare mask、select operands、单一 unmasked store、guard、
  chunk width、epilogue与 checked proofs。checker 不调用 discovery/materializer。
- LLVM lowering复用 `KirInstructionKind::{VectorLoad,VectorCompare,VectorSelect,
  VectorStore}` 的现有路径；只在发现真实缺口时修改
  `src/backend/llvm/kir_lower.rs`。
- 扩展 `tests/optimizer/vectorize.rs`、`tests/native/vector_llvm.rs` 和
  `tests/native/differential.rs`。

目标 KIR 形状固定为：

```text
old.v       = vector-load same-place
candidate.v = strict vector arithmetic
mask.v      = vector-compare lt candidate.v, old.v
chosen.v    = vector-select mask.v, candidate.v, old.v
memory.next = vector-store same-place, chosen.v
```

## TDD 顺序

1. 添加 RED `predicated_update_should_materialize_select_and_unmasked_store`，断言
   vector body 恰好一条 compare/select/store，select false operand 是同 place
   old vector load，store value 是 select result，且有 scalar epilogue。
2. 添加 checker mutation RED：交换 select 两臂、换 compare、mask 来源、store
   place/value/memory input、增加第二 store、删除 guard、篡改 VF/UF/minimum 或
   post digest，逐例必须返回 compiler-owned rejection。
3. 实现专用 schedule 与 materialization，保持现有 pure diamond
   `MergeMemory` fail-closed；运行 optimizer 正向用例转绿。
4. 实现独立 checker 重建，不共享 proposer 的 shape helper；先运行 mutation
   tests，再运行整个 vectorization transaction suite。
5. 添加 LLVM IR RED，要求 `fcmp olt`、`select <... i1>`、单一 vector store，且
   禁止 masked-store intrinsic/fast flags。构建并 verify module。
6. 添加真实 differential：scalar O0 与 tuned vector artifact 在 training/
   validation/adversarial matrices 上输出 bitwise-equal strict f64；checked 正向
   相同，proof 缺失版本保持 scalar。
7. 运行阶段命令并记录 stage-15 evidence。

## 阶段命令

```sh
cargo test --locked --test optimizer predicated_update_should_ -- --nocapture
cargo test --locked --test optimizer predicated_update_checker_ -- --nocapture
cargo test --all-features --locked --test native predicated_update_llvm_ -- --nocapture
cargo test --all-features --locked --test native predicated_update_differential_ -- --nocapture
cargo test --locked --test optimizer -- --nocapture
```

## 边界

- 不能用 masked store、fast math、speculative may-fail operation 或重排 effect。
- checker 必须独立重建，不能相信 candidate、plan mapping 或 attestation 文本。
- 不扩大到任意 conditional store、gather/scatter 或非 unit-stride 访问。
