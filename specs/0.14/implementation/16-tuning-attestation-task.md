# 阶段 16 任务：Loop SIMD 调优保留与 Source-Aware Attestation

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

让 Auto-Tuning 保留 predicated-update 的不同合法 VF/UF/minimum 变体，精确
replay 获选变体，并在 `--explain-optimization` 下输出由独立 checker 授权的
`CKTUNE-ATTEST/1`。不得增加 CKTUNE01 字段；诊断不能替代 source-aware 验证。

## 仓库落点与接口

- 修改 `src/optimizer/tune.rs`：现有 `TuneAlternativeClass::LoopSimd` 与
  `TuneAlternativePayload::LoopSimd { vector_bits, interleave,
  break_even_iterations }` 保持 wire 形状；candidate stable payload 加入阶段 14
  shape identity，且同一 site 的不同 VF/UF 不得被错误 dedup。
- 新增只读证明对象：

```rust
pub struct PredicatedUpdateAttestation {
    pub function: String,
    pub header: BlockId,
    pub compare: InstructionId,
    pub load: InstructionId,
    pub store: InstructionId,
    pub unit_id: [u8; 32],
    pub variant_id: [u8; 32],
    pub alternative_id: [u8; 32],
    pub vector_bits: u32,
    pub interleave: u32,
    pub minimum: u32,
    pub pre_state_digest: [u8; 32],
    pub post_state_digest: [u8; 32],
}
```

- 提供 `attest_selected_predicated_update(pre_state, space, plan)`：先运行完整
  `check_tuning_plan` 和独立 vector checker，再要求 exactly one PlanChoice、
  one UnitVariant、one SiteAlternative、target shape、minimum<=128，并返回上述
  只读事实；任何不满足返回稳定错误。
- 修改 `src/cli/tune.rs` 的 cold/warm `tune build` 与 `run_replay`，仅在
  `args.explain_optimization` 且 attestation 成功后向 stderr 写一条规范 line。
  tuned/replay 共用 formatter，但分别独立重建事实；普通 build/tune 和 stdout
  不变。
- 扩展 `tests/optimizer/tuning.rs`、`tests/cli/tune.rs`、
  `tests/tune/replay.rs` 与现有 decision golden/mutation tests；不新增测试 driver。

## TDD 顺序

1. 添加 RED：同一 predicated site 枚举至少两个合法 VF/UF variant，variant id、
   payload 和 isolated post-state 各不相同，frontier 不错误合并。
2. 添加 RED `predicated_attestation_should_require_exact_single_choice`：baseline、
   layout-only、复合 plan、第二 SIMD、wrong site、multi-alternative unit、
   minimum=129、篡改 pre/post/ids 全部拒绝。
3. 实现 attestation 重建函数；它只能消费 immutable pre-state、重枚举 space、
   checked plan，不读取 report 或信任 decision 文本字段。
4. 添加 CLI RED，冻结 exact ASCII field order、canonical decimal/lowercase digest、
   单 LF；普通命令无 `CKTUNE-ATTEST/`，tuned/replay 各恰好一条且 byte-equal。
5. 将 formatter 接到 cold、warm-cache publish 和 exact replay 三条成功路径；在
   publication 前完成检查，失败不产生 decision/artifact。
6. 添加 CKTUNE01 byte-for-byte golden 回归，证明无 tag/schema/size 变化；运行
   stale decision、wrong target/profile、post-state mutation negative。
7. 运行阶段命令并记录 stage-16 evidence。

## 阶段命令

```sh
cargo test --locked --test optimizer predicated_tuning_ -- --nocapture
cargo test --locked --test tune predicated_attestation_ -- --nocapture
cargo test --all-features --locked --test cli predicated_attestation_ -- --nocapture
cargo test --locked --test tune decision_ -- --nocapture
```

## 边界

- attestation 是诊断输出，不进入 CKTUNE01、cache key、语言或 ABI。
- 不允许复合 plan 通过独立 Floyd gate；通用 tuner 仍可为其他 workload 选择
  合法复合 plan。
- 不为获得单一 choice 删除全局搜索候选；gate 只接受真实搜索结果。
