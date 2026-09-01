# 阶段 04 验收：canonical loop、dependence 与 predicate

## 必须通过

1. `cargo test --locked --test optimizer loop_simplify_ -- --nocapture`
2. `cargo test --locked --test optimizer loop_descriptor_ -- --nocapture`
3. `cargo test --locked --test optimizer affine_access_ -- --nocapture`
4. `cargo test --locked --test optimizer dependence_ -- --nocapture`
5. `cargo test --locked --test optimizer version_predicate_ -- --nocapture`
6. `cargo test --locked --test ir loop_proof_ -- --nocapture`
7. `cargo test --locked`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --locked -- -D warnings`
10. `git diff --check`

过滤测试不得为 0 项。

## 结构断言

- Canonical descriptor 与 LCSSA 对嵌套/break/continue 稳定；不可约/预算耗尽不产生半规范 CFG。
- CFG mutation 后旧 descriptor/fact/proof 被拒绝，重建覆盖 dominance/Memory SSA/contracts。
- 每个潜在 loop-carried write pair 精确分类；unknown 不会因不同名字自动变 independent。
- Predicate overflow 只为 false，zero footprint 与无关 pointer comparison 规则正确。
- 本阶段 pipeline 不含 Vector KIR、specialization 或 unroll materialization。

## 完成证据

写入 `target/acceptance/v0.12/stage-04/`：实现 SHA、RED 摘要、loop/dependence/predicate
mutation count、fallback reason 列表、默认全仓测试计数。
