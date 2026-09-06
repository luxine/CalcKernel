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
Generation object audit 还必须证明 compiler-private initialization guard 是 `NoInline`，避免完整
runtime initialization 参数准备被复制进热插桩路径；5x 门槛、site/counter、batch、样本与 corpus
保持不变。Candidate-constant observation 必须使用 function-local saturated batching，最终 bucket
计数精确且 hot comparison 不含逐 observation atomic runtime call。AArch64 generic SVE/SVE2
multiversion member 必须使用固定 schedule-only tuning model，且 feature audit 仍证明无越权 ISA。
x86 Native target profile 必须在既有封闭 `UF <= 4` 候选空间内暴露至少四路 interleave；
`strict_f64` 与 `integer_cast` 必须选择经过 checker 的 `VF2` 多独立链计划，具体 `UF2/UF4` 由
真实 target cost 决定，并继续通过不变的 SIMD 性能门槛。
无 profile multiversion 必须使用确定性的 8-instruction pure-helper inline budget；普通 O3 的
32-instruction budget 与 PGO-hot 的 48-instruction budget 保持不变。仍被调用的 9..32 instruction
pure helper 必须在 Native member 中保持 `noinline`，小型 hot-path helper 仍 inline，且 object
cache identity 必须编码该策略。
三流 noalias integer map 在 x86 target cost 选择 `VF4/UF4` 时，必须在既有 aggregate `2x`
KIR growth 上限内物化；chunk start、完整 backedge advance 与 dominating MemorySSA reuse 均由
独立 checker 验证，机器码主循环每轮至少保留四条 128-bit 独立链。
selected-direct channel 必须从独立加载的同字节 multiversion artifact 读取并调用 resolver 实际
发布的 hidden member，只绕过 public thunk，不能再以 `--cpu native` 产物冒充 selected tier。
x86 constant-call scalar memory-map 必须使用 IR-semantic 1×5 schedule 且不影响 checked/reduction/
pre-vectorized loop；所有 multiversion physical module 必须携带 verified contract facts重新验证，缺失时
fail closed，Native fact audit不能绕过。

## CI 必须通过

- exact candidate SHA 的 quality、native integration、darwin-arm64、darwin-x64、linux-arm64、
  linux-x64、win32-arm64、win32-x64、x86-64 performance、AArch64 performance 共十个 required jobs
  全绿，无 skipped/continue-on-error/cancelled gate。
- quality 必须先通过 `SOURCE_MANIFEST.sha256` 校验和 frozen lockfile 构建仓库内固定 TypeScript
  oracle，再实际执行 C/WASM/CLI/fixture differential tests；不能访问私有退役仓库、替换成不兼容
  registry artifact 或跳过 oracle-only tests。
- performance workers 的 required enhanced tier/capability manifest存在；缺失不是 skip 条件。
- workflow run head SHA 精确等于最终 candidate SHA；若为记录证据又提交，必须对新 SHA 重跑。

## 完成证据

写入 `target/acceptance/v0.13/final/` 与 CI artifact：candidate/replay/toolchain identities、schema 8
report/checker digest、两架构 threshold summary、artifact/profile/variant/capability digests、run URL/id/
job conclusion。远程未完成时本阶段和总验收不得签署通过；不得回写动态证据制造新 SHA。
