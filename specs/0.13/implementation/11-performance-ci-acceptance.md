# 阶段 11 验收：schema 8 性能、size/time 与 exact-SHA CI

## 本地契约必须通过

1. `cargo test --locked --test performance -- --nocapture`
2. `python3 -m unittest discover -s tests/performance -p '*_test.py'`
3. `cargo test --locked --test contracts ci_ -- --nocapture`
4. `cargo fmt --check`
5. `cargo clippy --all-targets --all-features --locked -- -D warnings`
6. `cargo test --locked`
7. `cargo test --all-features --locked`
8. `cargo build --release --features native-toolchain --locked`
9. `scripts/test-sanitized-ownership.sh`
10. `scripts/audit-ckc-release.sh target/release/ckc`
11. `scripts/audit-native-artifact.sh target/native-acceptance/v0.13-final`
12. `scripts/audit-jit-memory.sh target/release/ckc`
13. `git diff --check`

## 稳定 worker 必须通过

在固定 x86-64 与 AArch64 workers、LLVM/Clang 22.1.8、Rust 1.90.0、exact 0.12 replay/capability
manifest 下分别执行：

14. `cargo bench --features native-toolchain --bench ckc_perf -- --case proof --task check --cpu baseline`
15. `cargo bench --features native-toolchain --bench pgo_perf -- --task collect --out target/ckc-perf/v0.13-results.json`
16. `python3 scripts/check-native-performance.py target/ckc-perf/v0.13-results.json`

两架构报告必须满足设计全部 cumulative thresholds：ordinary regression、PGO、multiversion、
dispatch direct、combined、Clang/Rust PGO oracle、0.12 SIMD/domain、generation overhead、artifact/
compile/archive size，且 schema/digest/sample/order/stability完整。

## CI 必须通过

- exact candidate SHA 的 quality、native integration、darwin-arm64、darwin-x64、linux-arm64、
  linux-x64、win32-arm64、win32-x64、x86-64 performance、AArch64 performance 共十个 required jobs
  全绿，无 skipped/continue-on-error/cancelled gate。
- performance workers 的 required enhanced tier/capability manifest存在；缺失不是 skip 条件。
- workflow run head SHA 精确等于最终 candidate SHA；若为记录证据又提交，必须对新 SHA 重跑。

## 完成证据

写入 `target/acceptance/v0.13/final/` 与 CI artifact：candidate/replay/toolchain identities、schema 8
report/checker digest、两架构 threshold summary、artifact/profile/variant/capability digests、run URL/id/
job conclusion。远程未完成时本阶段和总验收不得签署通过；不得回写动态证据制造新 SHA。
