# 阶段 06 任务：事实驱动函数专用化

## 目标

实现 internal direct-call specialization 的 scalar-only完整状态 transaction。用 dominating
exact constants/slice length/alignment/noalias/effect facts 创建 deterministic clone，满足实质
scalar 收益才提交，然后只经过一次正常 O2/O3 后续流水线。

## 仓库落点

- 新增 `src/optimizer/analysis/specialization.rs`、
  `src/optimizer/kir_passes/specialize.rs`。
- 扩展 contract fact clone/remap、proof checker、pipeline stats/explanations。
- 测试：`tests/optimizer/kir_o3.rs`、`alias_effects.rs`、`preservation.rs`，必要的 C/Wasm/
  Native ABI symbol tests。

## TDD 顺序

1. 写 candidate discovery RED：internal direct call、dominating fact、canonical fact-set digest、
   caller/call/callee/digest 稳定 key；storage 顺序不影响。
2. 写排除 RED：export/address-taken/indirect/runtime/recursive SCC/sanitizer 不专用化；trusted
   contract 只能用于其 dominated call instance。
3. 写 complete-state trial RED：substitute/clone/redirect 后只跑 clone-local CFG/SCCP/range/
   check/DCE；rejection 全回滚 verified state，但 caller/callee budget 与 reason 保留。
4. 写 independent checker RED：argument/ID/fact scope、scalar cost、growth、budget；伪造 clone
   mapping、跨 instance fact、错误 digest、trial-only audit ID 拒绝。
5. 写 profitability RED：至少 10% 且两个 absolute cost unit，不能以未来 vector 收益代替；
   exact threshold 边界正反例。
6. 写 limit/reuse RED：每 original function 最多 3 clone、module 最多 24、共享 growth allowance；
   相同 fact set reuse，不退款；clone 永不成为 specialization root。
7. 写 ABI/symbol RED：generic body 保留，export signature/thunk/header/dynamic symbol 不变，clone
   internal deterministic name；O2/O3 只处理 accepted clone 一次。
8. 用 fixed slice length/check elimination kernel 验证 accepted specialization 确有 scalar
   materialized saving，并为后续 vector stage 暴露常量 trip。

## 实现判定

- Acceptance 原子交换完整 `KirVerifiedProgramState`；audit 在外层单调更新。
- Trial 不运行 loop/unroll/vector/SLP，也不迁移 trial-only descriptor/proof。
- Sanitizer 下 pass record 可存在但 changed=false，并给稳定 reason。
- O0–O2 不启用 specialization；O3 位置严格在 O1 prefix 后、O2 inline 前。
