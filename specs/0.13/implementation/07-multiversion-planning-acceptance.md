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
- checker 独立复算 eligibility/profit/growth/shared budget/order；每 root 最多两个 enhanced，module
  total KIR <=2x baseline，拒绝/non-winner 不退款。
- 非 O3、unsupported consumer/sanitizer/object combination 在任何输出前明确失败。

## 完成证据

记录实现 SHA、target-set/variant digests、feature containment、budget mutation 与命令结果。阶段 07
未实现 production dispatcher，不能以 forced direct variant 代签 runtime selection。
