# 阶段 14 任务：Predicated Same-Place Update 分析与合法性模型

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

让 CK-owned vector discovery 识别严格形状
`old=load(place); candidate=...; if candidate<old { store(place,candidate) }`，
同时 fail-closed 拒绝不同地址、双臂写、intervening write、别名不明、ordered
effect、strict-f64 不一致和 checked proof 不完整。阶段只建立候选与独立可重建
证据，不物化向量 KIR。

## 仓库落点与接口

- 修改 `src/optimizer/analysis/vectorize.rs`，新增并导出：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorPredicatedUpdate {
    pub then_block: BlockId,
    pub else_block: BlockId,
    pub merge_block: BlockId,
    pub condition_instruction: InstructionId,
    pub old_load_instruction: InstructionId,
    pub store_instruction: InstructionId,
    pub store_when_true: bool,
    pub memory_input: MemoryVersionId,
    pub memory_output: MemoryVersionId,
}
```

- `VectorizationCandidate` 新增
  `predicated_update: Option<VectorPredicatedUpdate>`；与现有 pure
  `diamond`、`reduction` 互斥。CandidateKey stable text 加入 shape、compare、
  load、store 和 branch polarity，确保不同 scalar root 不能碰撞。
- `simple_shape` 拆成 pure-diamond 与 predicated-update recognizer。后者只接受
  五 block canonical loop、恰好一个 Store、另一臂无 memory/effect、merge 的
  memory phi 恰好选择 old/new version、value args 无第二个 varying merge。
- 用现有 `analyze_affine_loop_accesses`、Memory SSA、dominators、effect order 与
  dependence facts 证明 old load 和 store 的 `KirPlace` 完全相等，load 支配
  compare/store，中间没有可能重定义；load/store 均是 unit-stride、同 region、
  同 element type。
- strict `f64` 只接受 `MirCompareOp::Lt` 和 StrictFloat candidate arithmetic；
  不启用 fast-math、reassociation 或 contraction。checked mode 继续要求现有
  lane bounds/overflow/first-failure proof。
- 修改 `src/optimizer/mod.rs` 的受控 re-export，并扩展
  `tests/optimizer/vectorize.rs` 与 `tests/optimizer/tuning.rs`。

## TDD 顺序

1. 在 `tests/optimizer/vectorize.rs` 添加 RED
   `predicated_update_discovery_should_accept_same_place_update`，构造 canonical
   KIR map 并断言唯一 candidate 的 compare/load/store id、memory input/output、
   `store_when_true=true`、VF/UF variants 与 stable key。
2. 添加表驱动 RED
   `predicated_update_discovery_should_fail_closed_on_false_shapes`，逐项变异：
   `<=`、store old、store different index、else store、第二 store、call/print、
   intervening alias write、non-unit stride、missing memory phi、NaN-changing
   arithmetic。每例必须无 candidate 且有稳定 fallback reason。
3. 添加 checked-mode RED：完整 bounds/overflow/first-error proof 可发现；删除任一
   proof 后保持 scalar。确认失败来自当前 `simple_shape` 拒绝 memory arm。
4. 实现 `VectorPredicatedUpdate` 与 shape recognizer；先只让 shape/Memory SSA
   tests 转绿，不修改 vector materializer。
5. 将访问、alias、effect、strict-f64 与 checked proof 串入现有 legality；确认
   ordinary discovery 仍受 static profitability，tuning discovery 只放宽收益
   cutoff 而不放宽合法性。
6. 验证 CandidateKey、candidate ordering 与现有 pure diamond/reduction snapshots
   稳定；运行阶段命令并记录 stage-14 evidence。

## 阶段命令

```sh
cargo test --locked --test optimizer predicated_update_discovery_ -- --nocapture
cargo test --locked --test optimizer -- --nocapture
cargo test --locked --test optimizer tuning_ -- --nocapture
cargo test --locked --test contracts docs_v0_14_should_describe_only_the_implemented_optimizer_boundary -- --nocapture
```

## 边界

- 测量 profile 不能建立安全 proof；未知 alias/effect/dependence 必须拒绝。
- 不把 conditional store 降成 masked store；目标形状是后续 select+unmasked
  vector store。
- 不改变普通 O3 静态收益阈值，也不接受 general if-conversion。
