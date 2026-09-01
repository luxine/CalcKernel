# CK 0.12 第三轮阻断项复诊

日期：2026-09-01
输入：`design-adversarial-review-03.md`
结论：**B7 成立。**

## B7 复诊：成立

现有 0.11 pass manager 已把 verified KIR snapshot 与用户可见 statistics/explanations 放在
同一个结果对象中，但 0.12 的 speculative transaction 比 0.11 更强：normal rejection
不是 compiler error，且必须保留尝试成本与稳定 reason。因此不能简单复制并整体交换现有
结果对象。

修订定义两个状态层：

1. `KirVerifiedProgramState`：module、contract facts、proof arena、eliminated guards、
   verification cache、generation 与确定性 IR ID allocator。它可以 snapshot、验证、整体
   commit 或整体 rollback。
2. `KirOptimizationAuditState`：冻结的 proposal/checker budget ledger、attempt sequence、
   accepted/rejected counters、stable explanation 与 budget fallback。它只允许单调追加/扣减，
   不随 KIR transaction rollback。

每个 proposal 按固定顺序在开始/执行 checker 时直接从 outer audit ledger 扣实际 step；
rejection、reuse、non-winning frontier candidate 都不退款。Audit identity 使用 transaction
开始前稳定的 source/KIR identity 加 candidate kind/VF/UF，不引用 trial-only ID。接受时只
交换 verified program state，再向 audit 追加 accepted record；拒绝时丢弃 verified snapshot，
向 audit 追加 reason。Malformed proof/post-commit failure 仍是 compiler error，并 withholding
artifact。

该分层保持预算更严格且满足原子回滚，不改变门槛。完成双语修订后继续第四轮审查。
