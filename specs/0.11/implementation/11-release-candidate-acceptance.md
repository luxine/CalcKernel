# 阶段 11 验收：0.11 候选硬化

## 本地必须通过

使用 pinned LLVM/Clang 环境：

1. `cargo fmt --check`
2. `cargo clippy --all-targets --all-features --locked -- -D warnings`
3. `cargo test --locked`
4. `cargo test --all-features --locked`
5. `cargo build --release --features native-toolchain --locked`
6. Linux：`scripts/test-sanitized-ownership.sh`，执行 ASan+UBSan+LSan；Apple 本地只记录 capability unavailable，不替代 Linux CI 的必跑门（当前 Apple Clang 17 runtime 在 macOS 26.6.2 上连最小 C ASan 程序也无法完成初始化）。
7. `cargo test --locked --test optimizer generated_ -- --nocapture`
8. `cargo test --locked --test ir mutation_ -- --nocapture`
9. `cargo test --all-features --locked --test native fact_audit_ -- --nocapture`
10. `cargo bench --features native-toolchain --bench ckc_perf -- --case proof --task check --cpu baseline`
11. `python3 scripts/check-native-performance.py target/ckc-perf/results.json`
12. `scripts/audit-native-artifact.sh target/native-acceptance`
13. `scripts/audit-jit-memory.sh target/release/ckc`
14. `./target/release/ckc --version --verbose`
15. `./target/release/ckc licenses`
16. `git diff --check`

## 远程必须通过

- feature branch 上显式 `workflow_dispatch` 的 quality、native-integration、六个
  native-host、x86-64/AArch64 performance jobs 全绿。
- 每个 native-host 上传/记录 pre-LLVM fact audit evidence，并拒绝注入 mutation。
- performance artifacts 记录 pinned 0.10 digest/compiler identity，四组阈值全通过。
- 不允许 skipped/neutralized required job；重跑必须保留失败日志并说明非代码 flake 证据。

## 仓库判定

- `Cargo.toml`/lock/version tests 为 0.11.0；无 tag/Release。
- current docs English/zh-CN 同路径同契约；0.10 migration 作为兼容段保留而非当前标题。
- backend/CLI 无正式 optimized-MIR path；`emit-mir` bytes 仍兼容。
- `git status --short` 只包含预期提交前变更；无 target/build/Ai_repository/LLVM prefix。

## 完成证据

追加本地命令结果、远程 workflow run URL/commit、六 host job IDs、两架构 performance 摘
要与最终阶段 SHA。
