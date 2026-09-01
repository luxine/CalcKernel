# CK 0.12 设计第四轮对抗性审查

日期：2026-09-01
审查对象：第三轮修订后的双语 CK 0.12 设计
结论：**暂不通过；1 个确定性/候选覆盖阻断项。B7 已关闭。**

## 已关闭项复核

`KirVerifiedProgramState` 与 `KirOptimizationAuditState` 已明确分层。拒绝或未获胜的 proposal
只回滚 program/evidence snapshot，预算扣减、attempt、counter 和 stable reason 保留，且 audit
identity 不引用 trial-only ID。B7 关闭。

## 阻断项 B8：candidate 总顺序不覆盖 specialization，并漏掉独立 partial unroll

修订文本说 candidate 按 function/LoopId 与 kind 顺序枚举，但 specialization call 没有 LoopId，
而它又与同一 function 后续 loop proposal 共享冻结预算。预算可能耗尽时，没有跨阶段稳定
顺序就无法唯一复现哪个 candidate 得到分析机会。

同时 Controlled-unrolling 允许 partial unroll 靠自身 branch-cost 收益独立接受，O3 frontier
却只列 `scalar partial-unroll-plus-SLP`。这会让 Native consumer 丢失规范已经允许的 scalar-
only partial candidate，也使“optional SLP”与固定 kind key 不完整。

必须给 specialization、loop frontier、residual SLP 分别定义稳定 key 和明确 stage order；loop
frontier 必须同时容纳 full/partial unroll 的 scalar-only 与加 SLP 变体，并用 variant rank
完成总排序。

## 第四轮判定

B8 影响预算确定性与已承诺 transform 覆盖，复诊修订后继续第五轮完整审查。
