# 阶段 03 验收：事实、证明与标量分析

## 必须通过

1. `cargo test --locked --test ir proof_ -- --nocapture`
2. `cargo test --locked --test optimizer scalar_ -- --nocapture`
3. `cargo test --locked --test optimizer contract_fact_ -- --nocapture`
4. `cargo test --locked --test frontend contract_ -- --nocapture`
5. `cargo fmt --check`
6. `cargo clippy --all-targets --locked -- -D warnings`
7. `git diff --check`

## 结构断言

- 所有 FactId/ProofId 输出跨运行 byte-identical。
- modular unchecked case 不继承会被 wrap 破坏的数学结论。
- checked operation 在证明安全前保留 may-fail。
- fake analysis producer 的错误结论被独立 checker 拒绝。
- widening 必定在固定步数内结束，超预算只得到 unknown，不删除检查。

## 完成证据

- 日期：2026-08-29
- 实现提交：`6c3b27f`
- `cargo test --locked --test ir proof_ -- --nocapture`：7 passed，0 failed/ignored。
- `cargo test --locked --test optimizer scalar_ -- --nocapture`：6 passed，0 failed/ignored。
- `cargo test --locked --test optimizer contract_fact_ -- --nocapture`：3 passed，0 failed/ignored。
- `cargo test --locked --test frontend contract_ -- --nocapture`：21 passed，0 failed/ignored。
- `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、
  `git diff --check`：全部通过。
- 补充全仓验证：`cargo test --locked` 共 282 passed，0 failed/ignored；配置固定 LLVM
  环境后执行 `cargo clippy --locked --all-targets --all-features -- -D warnings` 通过。
- 8 类定向 mutation/拒绝路径：forward FactId、fake producer 错误常量结论、stale
  generation、错误 Proven/TrustedContract origin、缺失 call instance、KIR 改变后的预算
  identity、非封闭 loop invariant、非支配 fact use。
- domain property table 覆盖 4 种整数类型的 4 个 top-range case 与 12 个
  min/zero/max exact case，另含 congruence 奇偶组合、known-bits 匹配、affine
  coefficient/constant、signed/unsigned checked extremes、modular wrap 和 strict branch
  refinement。
- 预算 fallback 样例将固定 `max_steps=1`；分析稳定返回 `exhausted=true`，所有整数值
  为 `unknown`，重复 20 次 byte-for-byte 相同。正常循环按派生预算执行确定 widening，
  并严格完成配置的 2 次 narrowing；预算与当前 KIR identity 不一致时 checker 拒绝。
- unsafe contract 在函数入口、每个 call edge 与 recursive call edge 创建独立 instance；
  inline clone 使用 fresh instance 且只支配声明的 clone block，不生成 caller-entry fact。
