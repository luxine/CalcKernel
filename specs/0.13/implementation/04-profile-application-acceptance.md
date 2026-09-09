# 阶段 04 验收：profile application、confidence、cost 与 mapping

## 必须通过

1. `cargo test --locked --test pgo_analysis -- --nocapture`
2. `cargo test --locked --test profile application_ -- --nocapture`
3. `cargo test --locked --test optimizer profile_ -- --nocapture`
4. `cargo test --all-features --locked --test cli pgo_use_ -- --nocapture`
5. `cargo test --all-features --locked --test cli pgo_build_use_ -- --nocapture`
6. `cargo test --locked --test optimizer transaction_ -- --nocapture`
7. `cargo test --locked`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --all-features --locked -- -D warnings`
10. `git diff --check`

每个 filter 必须非零；Native CLI 用真实 final profile fixture，不能绕过 compatibility parser。

## 结构断言

- confidence/hotness/work/cost 只用固定整数规则与 checked arithmetic；饱和、溢出、歧义、tie
  都稳定回退 baseline，不能通过 wrapping、float 或代表 bucket value 决策。
- independent checker 从 record/profile/target formula 重算每个 class lower bound 与 net benefit，
  不信任 proposal total。
- CFG-changing transform 未提供 checked closed transfer 时 affected mapping 变 unknown；profile count
  不出现在 fact/proof arena。
- profile off 与未达到 confidence 的 use 保持 ordinary decision；explanation 精确且 deterministic。

## 完成证据

记录实现 SHA、identity/profile digest、边界/mutation 测试、off/use KIR digest 和命令结果。此阶段
仍不得用 explanation 代签实际 O2 layout 或 O3 transform。
