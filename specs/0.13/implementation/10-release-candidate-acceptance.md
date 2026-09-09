# 阶段 10 验收：0.13.0 identity、兼容与 current docs

## 必须通过

1. `cargo test --locked --test contracts release_ -- --nocapture`
2. `cargo test --locked --test contracts docs_ -- --nocapture`
3. `cargo test --locked --test contracts native_toolchain_ -- --nocapture`
4. `cargo test --locked --test compatibility -- --nocapture`
5. `cargo test --all-features --locked`
6. `cargo build --release --features native-toolchain --locked`
7. `target/release/ckc --version`
8. `target/release/ckc --version --verbose`
9. `scripts/audit-ckc-release.sh target/release/ckc`
10. `scripts/audit-native-artifact.sh target/native-acceptance/v0.13-stage-10`
11. `scripts/audit-jit-memory.sh target/release/ckc`
12. `cargo fmt --check`
13. `cargo clippy --all-targets --all-features --locked -- -D warnings`
14. `git diff --check`

每个 filter 非零；release/native/JIT audit 使用本阶段真实二进制/fixture artifact。

## 契约断言

- package/lock/CLI/docs 是 0.13.0；KIR 3、bridge 4、CKCOBJ03/key+manifest 4 与 private runtime/
  target/profile schemas 一致；Native ABI 1、Runtime ABI 2 未改变。
- 0.12 source/semantics/diagnostics/public ABI/runtime compatibility 全绿，旧 private schema fail-closed。
- 英中 current docs 覆盖普通无训练、自动/显式 PGO、multiversion、限制、安全/隐私、性能契约，
  不把 0.14 Auto-Tuning/future boundary 写成已实现。
- release workflow/archives 保持 pinned、六平台、自包含、license/provenance/SHA contract；未创建 tag/Release。

## 完成证据

记录 candidate SHA、binary/archive SHA、verbose identity、compatibility counts、docs parity 与 audit 输出。
阶段 10 不能代签 schema 8 性能或 exact-SHA CI。
