# 阶段 06 验收：事实驱动函数专用化

## 必须通过

1. `cargo test --locked --test optimizer specialization_ -- --nocapture`
2. `cargo test --locked --test optimizer specialization_scope_ -- --nocapture`
3. `cargo test --locked --test optimizer specialization_budget_ -- --nocapture`
4. `cargo test --locked --test optimizer specialization_abi_ -- --nocapture`
5. `cargo test --locked --test optimizer sanitizer_ -- --nocapture`
6. `cargo test --locked`
7. `cargo fmt --check`
8. `cargo clippy --all-targets --locked -- -D warnings`
9. `git diff --check`

过滤测试不得为 0 项。

## 结构断言

- Accepted clone 有真实 constant/check/CFG simplification，非仅重命名；低于阈值完全回滚。
- Trusted fact 不跨 call instance，clone/reuse/limit/budget mutation 均由独立 checker 复诊。
- Rejected trial 仍扣双侧 budget；audit 不引用 trial-only ID。
- Generic/export/header/ABI symbols 不变，clone internal 且不递归专用化。
- Pipeline 中 clone 不提前或重复运行 loop/vector transforms。

## 完成证据

写入 `target/acceptance/v0.12/stage-06/`：实现 SHA、RED 摘要、accepted/rejected/reused/limit
counts、threshold boundary cost、ABI symbol audit 与默认全仓 test count。
