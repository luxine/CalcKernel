# 阶段 02 验收：Vector/Mask KIR 与 closed plan schema

## 必须通过

1. `cargo test --locked --test ir vector_type_ -- --nocapture`
2. `cargo test --locked --test ir vector_instruction_ -- --nocapture`
3. `cargo test --locked --test ir vector_memory_ -- --nocapture`
4. `cargo test --locked --test ir vector_proof_ -- --nocapture`
5. `cargo test --locked --test optimizer vector_plan_ -- --nocapture`
6. `cargo test --locked`
7. `cargo fmt --check`
8. `cargo clippy --all-targets --locked -- -D warnings`
9. `git diff --check`

过滤测试不得为 0 项。

## 结构断言

- Scalar KIR 全部显式使用 `KirValueType::Scalar`，function/public ABI 类型未迁移为 vector。
- 每个 vector family 有正例和至少一个独立 mutation 负例；validator 不调用未来 proposer。
- Vector memory footprint/Memory SSA/alignment/profile identity 全部 fail-closed。
- C/Wasm 不悄悄 scalarize 手工构造的 Vector KIR；Native rejection/lowering 由阶段 03 验收；
  正常 pipeline 仍只产 scalar。
- 默认全仓测试保持通过，无 ignored test。

## 完成证据

写入 `target/acceptance/v0.12/stage-02/`：实现 SHA、RED 摘要、vector instruction/validator
mutation 数、scalar regression count。
