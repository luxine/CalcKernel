# 阶段 09 验收：0.12.0 release identity 与 current docs

## 本地必须通过

在固定 Native toolchain 下：

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features --locked -- -D warnings`
3. `cargo test --locked`
4. `cargo test --all-features --locked`
5. `cargo build --release --features native-toolchain --locked`
6. `./target/release/ckc --version --verbose`
7. `./target/release/ckc --help`
8. `./target/release/ckc licenses`
9. `scripts/audit-ckc-release.sh target/release/ckc`
10. `scripts/audit-native-artifact.sh target/native-acceptance`
11. `scripts/audit-jit-memory.sh target/release/ckc`
12. `git diff --check`

Windows 对应审核由 exact-SHA CI 的 PowerShell scripts 完成。

## 契约断言

- Cargo/lock/compiler/current docs 为 0.12.0；无残留“0.11 current”或误称 PGO/autotune 已完成。
- Native ABI=1、Runtime ABI=2、bridge ABI=3、KIR v2、cache `CKCOBJ02`/schema 3。
- v0.11 compatibility fixtures 在 source/observable/public ABI 范围通过；旧 cache/bridge identity
  明确拒绝。
- 双语 current docs、CLI help、release policy、workflow asset names 一致。
- 阶段 09 本地完整回归全绿；最终 performance/exact-SHA 十作业由阶段 10 验收。无 tag/
  Release/merge。

## 完成证据

写入 `target/acceptance/v0.12/stage-09/`：阶段 SHA、版本/ABI 输出、default/all-feature/
release counts、audit摘要、`git status`、`git worktree list` 和 main SHA；阶段 10 的最终远程
证据不得回写仓库改变最终 SHA。
