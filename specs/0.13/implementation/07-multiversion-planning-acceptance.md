# 阶段 07 验收：target set、variant planner/checker 与 KIR bundle

## 必须通过

1. `cargo test --locked --test multiversion planning_ -- --nocapture`
2. `cargo test --locked --test optimizer multiversion_ -- --nocapture`
3. `cargo test --all-features --locked --test native target_set_ -- --nocapture`
4. `cargo test --all-features --locked --test native variant_feature_ -- --nocapture`
5. `cargo test --all-features --locked --test cli multiversion_ -- --nocapture`
6. `cargo test --locked --test contracts kir_ -- --nocapture`
7. `cargo test --locked`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --all-features --locked -- -D warnings`
10. `git diff --check`

每个 filter 非零；target-set contract 至少用 fixtures 覆盖六个平台组合，本机支持项再走真实 LLVM。

## 结构断言

- target set 是 closed canonical table；x86 v3/v4 与 Linux AArch64 SVE/SVE2 的硬件+OS state predicate
  完整，Darwin/Windows AArch64 schema 1 baseline-only。
- 所有 accepted variant 从同一 baseline logical pre-state，separate LLVM module、独立 proof/cost/
  feature digest、hidden symbols；无 cross-variant LTO。
- checker 独立复算 eligibility/profit/growth/shared budget/order；每 root 最多两个 enhanced，unique
  normalized KIR-body total <=2x baseline。仅 profile/tier hidden name 不同且规范化后逐字节一致的
  member 共享 body charge，真实结构差异完整计费；独立 target module/object/audit 与 artifact gate
  不共享，拒绝/non-winner 不退款。
- enhanced retained-set 必须有一个通过不变 profitability floor 的 trial 作为 eligibility witness；
  profitable strict superset 的 required-feature subset 只有在 predicted cost 不差于 baseline 时，
  才可作为 compatibility companion 进入 retained-set。若 shared full-root budget 只容纳一个 enhanced
  member，先保留 required feature 更少、runtime 兼容覆盖更广的候选；witness 无需同时物化，再按
  dynamic cost、size、tier 与 root identity 决胜。
- 非 O3、unsupported consumer/sanitizer/object combination 在任何输出前明确失败。

## 完成证据

记录实现 SHA、target-set/variant digests、feature containment、budget mutation 与命令结果。阶段 07
未实现 production dispatcher，不能以 forced direct variant 代签 runtime selection。
