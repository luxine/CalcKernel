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
- performance artifacts 使用 schema 5，记录 pinned 0.10 digest/compiler identity、每项
  `v010MedianNs`/`v010ClangMedianNs` 与冻结 C-oracle digest；配对归一化后的四组阈值全通过。
- 不允许 skipped/neutralized required job；重跑必须保留失败日志并说明非代码 flake 证据。
- Windows job 的 bootstrap/compiler/archive identity 必须为 MSVC/`.lib`。Darwin JIT audit
  必须接受且只接受与 runtime capability 一致的安全 tuple：`map-jit=yes / thread-wx-supported=yes / thread-wx=yes`，
  或 `map-jit=no / thread-wx-supported=no / thread-wx=no`；两者共同满足
  relocation=RW/NX、code=RX、data=NX，不得出现 RWX。
- Cache key 必须随任一 runtime 编译/链接输入变化，且成功 bootstrap 即保存验证过的
  release/oracle prefix。Darwin host 必须执行经 `__ck_start` 进入的 standalone executable；
  x86-64 不得把 LLVM C-ABI `main` 直接用作 raw `LC_MAIN` entry。
- Linux artifact audit 必须同时拒绝 loader-visible dependency/undefined executable symbol/
  unexpected export，并只在 `.comment` 为 non-`ALLOC` 且包含 pinned LLD 22.1.8 marker 时接受
  provenance。Darwin entitlement 必须与仓库唯一 `allow-jit=true` policy canonical 等值。
- `CALCKERNEL_TS_ROOT` 在 CI workflow 只属于实际 checkout/build oracle 的 quality job；Native
  jobs 的 CLI suite 在无该变量时完整通过，不能指向不存在的目录。

## 仓库判定

- `Cargo.toml`/lock/version tests 为 0.11.0；无 tag/Release。
- current docs English/zh-CN 同路径同契约；0.10 migration 作为兼容段保留而非当前标题。
- backend/CLI 无正式 optimized-MIR path；`emit-mir` bytes 仍兼容。
- `git status --short` 只包含预期提交前变更；无 target/build/Ai_repository/LLVM prefix。

## 完成证据

追加本地命令结果、远程 workflow run URL/commit、六 host job IDs、两架构 performance 摘
要与最终阶段 SHA。

首轮候选 CI 的真实阻断、原始 job 与不降门槛的修订边界见
`../review/implementation-blockers-01.md`；必须在修复后的完整 matrix 全绿后补写复审结论。
