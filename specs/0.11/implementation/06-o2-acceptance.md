# 阶段 06 验收：O2

## 必须通过

1. `cargo test --locked --test optimizer kir_o2_ -- --nocapture`
2. `cargo test --locked --test optimizer unsafe_inline_ -- --nocapture`
3. `cargo test --locked --test optimizer gvn_ -- --nocapture`
4. `cargo test --locked --test optimizer memory_opt_ -- --nocapture`
5. `cargo test --locked --test optimizer runtime_print_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- O2 exact pass order 与规范一致。
- unsafe inline scope mutation 被 verifier 拒绝，递归 edge 使用新实例。
- third-root alias、unknown call 与 memory phi 均阻止不合法 GVN/forward/DSE。
- checked failure 与 print 的数量和顺序跨 inline/cleanup 不变。
- O2 新删 guard 均有通过独立 checker 的 ProofId。

## 完成证据

执行时追加 SHA、inline budget、Memory SSA rewrite 数和负向 case 摘要。

## 执行记录（2026-08-29）

- 实现提交：`95fd97c3b076631f3702d5cf24908820cee8c89c`
- 固定 pipeline：12 项 pass record 与本文结构断言完全一致，每项 `verified=true`。
- inline budget：单 callee 最多 32 条 KIR instruction，单 module 最多 128 次 clone；
  value/void/multi-return 三次连续 clone 通过，大函数保留 call。
- unsafe inline：每次 clone 生成 fresh `InlineClone` contract instance，facts 只覆盖 clone
  blocks；将其 scope 扩大为 caller entry 的 mutation 被独立 checker 拒绝。
- GVN：strict-f64 完全相同 key 合并 1 次；支配 block-param 等价合并通过；反向操作数、
  checked arithmetic 和非等价邻例均不合并。
- Memory SSA：正例 forward load 1 次、删除被覆盖 store 1 次；third-root alias、callee
  write call、join memory phi、print 与 may-fail 屏障均保持原 load/store。
- 二次检查消除：inline 恢复原 callee 已删除的 guard，再在 caller clone 上生成新的有效
  `ProofId` 后删除，未复用旧 rewrite identity。
- 本文件“必须通过”第 1–8 项全部通过；另执行 `cargo test --locked`，默认特性全仓
  319 个测试通过。
