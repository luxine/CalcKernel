# 阶段 10 验收：schema 7 性能与最终 CI

## 本地契约必须通过

在固定 Native toolchain 下：

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
11. `scripts/audit-native-artifact.sh target/native-acceptance`
12. `scripts/audit-jit-memory.sh target/release/ckc`
13. `git diff --check`

## 稳定 worker 必须通过

在 x86-64 与 AArch64 baseline、pinned LLVM/Clang/Rust 和完整 replay bundle 下：

14. `cargo bench --features native-toolchain --bench ckc_perf -- --case proof --task check --cpu baseline`
15. `python3 scripts/check-native-performance.py target/ckc-perf/results.json`

两架构报告都必须满足设计中的全部 cumulative threshold，且 schema/digest/sample/order 完整。
Optimizer latency remediation 还必须通过 exact-profile memoization invalidation、CFG-only dominance
identity/budget debit、incremental module-global identity collision、no-op frontier loop-analysis
function-identity fallback 与 unreported mutation 测试；不得以减少 verifier/checker 覆盖来通过门槛。

性能失败或显式 diagnostic 模式还必须在同一 worker 对 schema 7 报告中的 48 个 scalar 动态库
逐一核对整库 SHA 并反汇编：32 个 candidate/current/replay-Clang measured artifacts、8 个 v0.11
replay Native artifacts、8 个 v0.10 replay Native artifacts。诊断不得重新构建、重新计时、读取旧
`runtimeReplay` 字段或替代 required performance gate；重复执行不得把上一轮哈希追加进本轮证据。

## CI 必须通过

- exact candidate SHA 的 quality、native integration、六 host、x86-64/AArch64 performance 共十
  个 required jobs 全绿，无 skipped/continue-on-error gate。
- feature branch workflow_dispatch 的 run id/SHA 与本阶段提交一致；若为记录证据又提交，必须
  对新 SHA 重跑。

## 完成证据

写入 `target/acceptance/v0.12/final/` 和 CI artifact：实现 SHA、schema mutation tests、
两架构 report/checker digest、阈值摘要、CI run URL/run id/job conclusion。远程未完成时本阶段
不得签署通过；不得为回写这些动态值产生新 SHA。
