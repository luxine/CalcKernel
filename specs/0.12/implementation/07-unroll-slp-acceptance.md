# 阶段 07 验收：受控展开与 SLP

## 必须通过

1. `cargo test --locked --test optimizer full_unroll_ -- --nocapture`
2. `cargo test --locked --test optimizer partial_unroll_ -- --nocapture`
3. `cargo test --locked --test optimizer slp_ -- --nocapture`
4. `cargo test --locked --test optimizer unroll_slp_transaction_ -- --nocapture`
5. `cargo test --locked --test optimizer unroll_checker_ -- --nocapture`
6. `cargo test --locked --test backend scalar_unroll_ -- --nocapture`
7. `cargo test --locked`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --locked -- -D warnings`
10. `git diff --check`

过滤测试不得为 0 项；如新增 `tests/backend.rs`，必须接入现有 C/Wasm support 而非复制执行器。

## 结构断言

- Full/partial/combined threshold 和 trip/body/factor/growth 边界均有正反例。
- Independent checker 覆盖 coverage/order/phi/LCSSA/remainder/cost/budget mutation。
- SLP 只做 identity pack，memory footprint 精确，不越 barrier。
- C/Wasm 只有 scalar unroll；Native 在阶段 07 不先于 Loop SIMD commit alternative。
- Rejection/non-winner/budget exhaustion verified state 原子回滚且 audit 计费。

## 完成证据

写入 `target/acceptance/v0.12/stage-07/`：实现 SHA、RED 摘要、各 factor/pack/negative mutation
count、C/Wasm differential 结果。
