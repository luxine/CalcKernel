# 阶段 06 任务：测量调度、双 validation 与 decision assembly

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

把完整 finalist 集按 session-digest 确定性调度到 smoke、search 和两轮 validation，保留 exact raw streams，
用 checked integer Q32/stability/paired-win 选择 certificate 或 baseline reason，并组装可由阶段 01 检查的决策。

## 仓库落点与接口

- 新建 `src/tune/{measure.rs,selection.rs,session.rs,environment.rs}`。
- `MeasurementScheduler` 只接收冻结 baseline/finalist 列表、case/calibration/session digest；公开
  `run_smoke`、`run_search`、`run_validation_round(1|2)`，其 event log 由 canonical coordinate 驱动。
- `derive_search_entrants` 与 `derive_selection` 使用 checked `u128`，返回完整 RoundSummary/outcome matrix/
  optional Certificate；`assemble_decision` 在 encode 前调用 self-contained 与 source-aware checker。
- 扩展 `tests/tune/{measurement.rs,selection.rs,session.rs}`，mutation tests 从真实 event log 删除/交换/
  插入 stream、row、call、timeout coordinate。

## TDD 顺序

1. 写 session digest/order RED：digest 只含规范列出的 measurement-independent 输入；phase/round/row/case
   permutation key 与 case rotation 使用 exact domain/framing/BE u64，baseline first、plan digest order。
2. 写 smoke/search RED：size-valid finalist 全部按 plan/case 顺序 smoke；3 warmup+20 measured，每 measured
   channel exactly 3 calls、store min；timeout 后保留 exact earlier complete-stream set，survivor order 不变。
3. 写 stability/score RED：upper median index 10、inclusive 80..120%、至少 16/20、weighted ceil Q32、
   checked u128；任一 required unstable stream abort，不得 selective rerun。
4. 写 entrant RED：search score→artifact bytes→choice count→plan digest total order，preset entrant bound；
   timed-out/search-nonwinner/compiled-unmeasured 状态从完整 source-aware finalist 集派生。
5. 写双轮 validation RED：phase 5/7 独立 rotations；每轮 <=97%、每 case <=102%、paired wins >=16；
   RoundPlan/CaseMedian/aggregate/rank/threshold 全部从 raw rows 重算。
6. 写 selection table RED：no entrant、empty Q、same winner、different winner 四行互斥完备；certificate 仅 tuned，
   timed-out 保留，其他 entrant outcome 精确。
7. 写 abort/publish boundary RED：baseline/runner/protocol/correctness/instability/incomplete validation/replay/
   wall partial 均不产生新 decision/artifact；完整且无收益时成功发布 baseline-selection decision。
8. 运行 `cargo test --test tune measurement_ -- --nocapture`、`selection_`、`session_` 和完整 locked tests。

## 实现边界

- 内部 decision measurement 每 evaluation 三次，与阶段 10 external release benchmark 的七批次严格分离。
- 时间值允许两次真实 cold session 不同；choice identity/plan/object/link/published bytes 必须相同。
- 没有任意 retry、sample clipping、floating selection 或结果观察后的 workload 排除。

