# v0.14 实现期设计修正 01：空计划的最终状态身份

## 阻断复诊

实现 source-aware replay 时发现，原 `decision-schema-1.md` 同时规定：

1. 空计划执行未经改变的 v0.13 O3 收益决策，并与普通 O3 生成结果逐字节一致；
2. 空计划的 `Replay.selectedPreState` 与 `Replay.selectedPostState` 都等于
   specialization 之前的 pre-tune KIR。

当普通 O3 提交 specialization、inlining、Loop SIMD、unroll 或 SLP 中任一改写时，
pre-tune KIR 与普通 O3 最终 KIR 必然不同，因此两条要求不能同时满足。这是会让合法实现和
独立 checker 得出不同结论的阻断性规范矛盾，不是验收门槛问题。

## 决议

- `Replay.selectedPreState` 始终是不可变 pre-tune KIR 身份；
- 空计划的 `Replay.selectedPostState` 是从该 pre-tune 状态重新执行普通 v0.13 O3
  收益决策及所有强制 bridge/cleanup 后的最终 KIR 身份；
- 非空计划仍记录逐选择相邻的 pre/post 状态，最后一个 post 是完成余下强制流水线后的最终 KIR；
- baseline trial、最终重放和 `--tune-use` 都独立重算该最终状态，不信任 decision 中的摘要；
- “空计划与普通 O3 产物一致”的既有阶段 03/04 验收保持不变，未降低任何门槛。

此修正只消除身份定义矛盾，不改变语言/ABI、安全语义、搜索预算、收益阈值或性能门槛。
