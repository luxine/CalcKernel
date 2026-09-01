# 阶段 05 验收：transaction、audit ledger 与独立 checker

## 必须通过

1. `cargo test --locked --test optimizer transaction_ -- --nocapture`
2. `cargo test --locked --test optimizer audit_ledger_ -- --nocapture`
3. `cargo test --locked --test optimizer candidate_order_ -- --nocapture`
4. `cargo test --locked --test optimizer independent_checker_ -- --nocapture`
5. `cargo test --locked --test optimizer verifier_cache_ -- --nocapture`
6. `cargo test --release --locked --test optimizer independent_checker_ -- --nocapture`
7. `cargo test --locked`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --locked -- -D warnings`
10. `git diff --check`

过滤测试不得为 0 项。

## 结构断言

- Rejected/reused/non-winning/budget-exhausted trial 的 verified state byte-for-byte 回滚，audit
  扣减不回滚。
- False plan/post-state 在 debug/release 都拒绝且无 artifact。
- Candidate 全序和解释输出对 storage/hash 顺序不敏感；无重复 key。
- Checker source 不调用 proposer、dependence analyzer 或 proposer cost total helper。
- 真实 production pipeline 尚无 specialization/unroll/vector mutation，O0–O2 不变。

## 完成证据

写入 `target/acceptance/v0.12/stage-05/`：实现 SHA、RED 摘要、mutation count、初始/消耗/
剩余预算向量、debug/release 命令结果。
