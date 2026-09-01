# CK 0.12 设计第三轮对抗性审查

日期：2026-09-01
审查对象：第二轮修订后的双语 CK 0.12 设计
结论：**暂不通过；1 个事务状态阻断项。B5-B6 已关闭。**

## 已关闭项复核

- Scalar full/partial unroll 与 unroll-plus-SLP 已具有 10% 加两个 cost unit 的明确门槛，
  Loop SIMD 仍为 20%；checker 可独立重算 accepted frontier。B5 关闭。
- Native profile probe universe 已固定为五种 lane type 乘 `{2,4,8,16}`、总宽度不超过
  512 bit，全集每项必须显式 Legal/Unavailable。B6 关闭。

## 阻断项 B7：trial 回滚与拒绝尝试永久计费相互矛盾

专用化段落把 statistics、explanations 和 frozen proposal/checker budgets 都放入“complete
verified optimization state”副本，并要求 rejection 时丢弃整个副本；预算段落又要求被
拒绝或复用的 trial 仍扣 caller/callee budget，explanation/statistics 段落也要求记录稳定
rejection reason。若完全丢弃副本，拒绝尝试没有成本和审计记录；若把副本整体写回，又会
把 trial 中未接受的 module/fact/proof mutation 一并泄漏。

这同样影响 loop frontier：未获胜 proposal 的分析/checker 工作必须永久耗费共享预算，
但它的 KIR mutation 不得提交。设计必须把可回滚的 verified program/evidence state 与不可
回滚的 attempt audit/budget ledger 分层，并规定接受/拒绝时各自如何更新。

## 第三轮判定

B7 是明确逻辑矛盾，必须复诊修订。修订时不得让拒绝尝试退还预算，也不得允许 audit
记录引用 trial-only、已丢弃的内部 ID。修订后继续下一轮完整审查。
