# 阶段 09 验收：0.14.0 identity、兼容与双语文档

## 必须通过

- [ ] `cargo test --test compatibility --locked`
- [ ] `cargo test --test contracts --locked`
- [ ] `cargo test --locked`
- [ ] `cargo test --all-features --locked`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features`

## 契约断言

- [ ] 0.14.0、CKCOBJ04/cache 5、五类 tune schema 1 在代码/CLI/docs/checker 中唯一一致。
- [ ] language/KIR/Native/Runtime/bridge/profile/multiversion ABI 保持冻结值，v0.13 ordinary behavior 无回归。
- [ ] schema4 cache clean miss、旧 profile identity mismatch、future tune schema fail-closed 有正负测试。
- [ ] 英中 current docs 路径和语义同步，canonical repository 为 `https://github.com/luxine/CalcKernel`。
- [ ] release/native artifact audit 证明 final executable/dynamic library 无 runner、tune symbol 或新增运行依赖。

## 完成证据

记录被测 SHA、版本矩阵、兼容 fixtures、文档同步检查、release audit 和测试计数到 `target/acceptance/v0.14/stage-09/`。

