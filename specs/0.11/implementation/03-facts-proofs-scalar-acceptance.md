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

执行时追加 SHA、mutation 数量、domain property case 数与预算 fallback 样例。
