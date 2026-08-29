# 阶段 05 验收：O0/O1

## 必须通过

1. `cargo test --locked --test optimizer kir_o0_ -- --nocapture`
2. `cargo test --locked --test optimizer kir_o1_ -- --nocapture`
3. `cargo test --locked --test optimizer guard_ -- --nocapture`
4. `cargo test --locked --test ir proof_ -- --nocapture`
5. `cargo test --locked --test optimizer runtime_print_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- O0 KIR 与 builder 输出除验证记录外相同。
- O1 pass order 与规范逐项一致，每项后都有 verifier record。
- 正例 guard 带有效 ProofId 消失；每个近邻反例保留并有确定 reason。
- checked 首错、print 和 may-fail mutation 全部拒绝非法 reorder/delete。
- 任一 invalid certificate 使 compilation failure，且 output transaction 未提交。

## 完成证据

执行时追加 SHA、每类 eliminated/retained guard 数与 mutation 结果。
