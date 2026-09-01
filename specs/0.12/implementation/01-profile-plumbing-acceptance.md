# 阶段 01 验收：KIR v2 profile 与 consumer plumbing

## 必须通过

1. `cargo test --locked --test ir profile_ -- --nocapture`
2. `cargo test --locked --test ir kir_v2_ -- --nocapture`
3. `cargo test --locked --test optimizer profile_ -- --nocapture`
4. `cargo test --locked --test cli emit_kir_consumer_ -- --nocapture`
5. `cargo test --locked --test cli optimization_level_ -- --nocapture`
6. `cargo test --locked`
7. `cargo fmt --check`
8. `cargo clippy --all-targets --locked -- -D warnings`
9. `git diff --check`

过滤测试不得为 0 项。

## 结构断言

- Portable C/default Inspection 使用 unknown layout，Wasm 使用固定 layout，三者都无 Legal
  vector operation。
- Profile canonical bytes/digest 有独立 mutation tests，缺失/重复/非法 entry 拒绝。
- KIR module/profile mismatch withholding artifact；O0 也不绕过检查。
- `emit-kir` consumer 与现有五种 `KirConsumer` 一一对应，不自动从源码猜 library/executable。
- 0.11 scalar semantics、pass count、diagnostics 与 default-feature 全仓回归无变化。

## 完成证据

写入 `target/acceptance/v0.12/stage-01/`：实现 SHA、RED 摘要、profile test vectors/digests、
各命令 exit code/test count。Native 尚未验收，不能在本阶段标记 bridge/profile/cache 通过。
