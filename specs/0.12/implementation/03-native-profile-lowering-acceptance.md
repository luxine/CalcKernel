# 阶段 03 验收：LLVM TTI profile、Vector lowering 与 cache/bridge v3

## 必须通过

在固定 `CKC_LLVM_PREFIX` 下执行：

1. `cargo test --all-features --locked --test native bridge_ -- --nocapture`
2. `cargo test --all-features --locked --test native target_profile_ -- --nocapture`
3. `cargo test --all-features --locked --test native vector_llvm_ -- --nocapture`
4. `cargo test --all-features --locked --test native fact_audit_ -- --nocapture`
5. `cargo test --all-features --locked --test native cache_schema_ -- --nocapture`
6. `cargo test --all-features --locked`
7. `cargo clippy --all-targets --all-features --locked -- -D warnings`
8. `cargo fmt --check`
9. `scripts/test-sanitized-ownership.sh`
10. `git diff --check`

过滤测试不得为 0 项。

## 结构断言

- 同 TargetMachine profile bytes/digest 重复一致，所有 universe key 恰一次。
- 手工 verified Vector KIR 在 pre-LLVM IR 中有真实 vector type/op；非法 vector/profile mutation
  在 LLVM pass 前拒绝。
- strict f64 无 fast flags/FMA contraction，alignment 不超过 proof。
- baseline/native feature containment、actual object parse 与 ownership sanitizer 通过。
- 旧 cache entry fail-closed，新 key 覆盖所有 object-affecting 0.12 identity。

## 完成证据

写入 `target/acceptance/v0.12/stage-03/`：实现 SHA、RED 摘要、Rust/host/LLVM manifest/
bridge ABI、profile digest、测试计数和 sanitizer exit code。本阶段结构 vector lowering
不等于自动 vectorization 通过。
