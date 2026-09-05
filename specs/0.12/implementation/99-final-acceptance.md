# CK 0.12 总验收

本文件是 0.12 候选的唯一总验收清单。阶段 acceptance 只证明局部完成，不能替代本清单。
所有复选项必须由同一最终 candidate SHA 的真实结果支持；不允许 ignored test、可选化
required job、旧日志、历史数值或 lowered threshold。

## A. 仓库与版本身份

- [ ] 当前分支为 `feature/v0.12-vector-optimizer`，worktree 为独立
  `.worktrees/v0.12-vector-optimizer`，工作区干净。
- [ ] `main` 未由本任务自动合并；未创建/移动 tag，未创建 GitHub Release。
- [ ] Cargo/lock/`ckc --version --verbose` 为 0.12.0；LLVM 22.1.8、bridge ABI 3、Native ABI 1、
  Runtime ABI 2 与 build target/manifest identity 正确。
- [ ] KIR print/schema 为 v2；Native object/run cache 是 `CKCOBJ02`、key/manifest schema 3；
  0.11 cache entry fail-closed。

## B. 语义与兼容性

- [ ] 0.11 全部 language/diagnostic/CLI/artifact/runtime/sanitizer/differential/ABI contract tests
  无回归，v0.11 compatibility fixtures 通过。
- [ ] Strict f64 无 contraction/reassociation/fast math；checked overflow/bounds 保持首错与之前
  已发生写入；print/runtime/effect 顺序与 O0 一致。
- [ ] Public Native headers、symbols、calling convention、slice/struct shape 保持 ABI 1；vector 与
  specialization clone 不 export。
- [ ] C/Wasm/default inspection 不含 Vector KIR；sanitizer 禁用所有 0.12 code-duplicating/vector
  transform，仍执行原 instrumentation。

## C. Profile、KIR 与验证

- [ ] `KirTargetProfile` schema 1 的固定 universe、TTI throughput/legalization/cost normalization、
  canonical bytes/SHA-256 digest 有 deterministic 与 mutation tests。
- [ ] `KirValueType`、全部 Vector/Mask instruction family、memory footprint/Memory SSA/alignment、
  region escape 与 consumer/profile validator 有正例和 mutation 负例。
- [ ] `KirVerifiedProgramState` 原子 commit/rollback；`KirOptimizationAuditState` 单调计费；拒绝/
  reuse/non-winner 不退款且不泄漏 trial KIR。
- [ ] Proposer 与 checker 分离；false cost/growth/budget/profile/lane/dependence/fallback/proof 在
  debug/release 都被独立 checker 拒绝并 withholding artifact。

## D. 优化功能

- [ ] Loop-simplify/LCSSA、descriptor invalidation、trip/induction、affine access/dependence 和 total
  version predicate 完整；irreducible/unknown/effect/budget 均保守 fallback。
- [ ] Specialization 只接受 materialized scalar >=10% 且 >=2 unit，fact scope/reuse/limits/growth/
  双侧预算正确；trial 不提前运行 loop/vector。
- [ ] Scalar full/partial unroll 与 unroll+SLP 满足 trip/body/factor/growth 和 >=10%+2 unit；SLP
  只做 identity pack且不跨 barrier。
- [ ] Loop SIMD 满足 >=20%，支持冻结的 element-wise/reduction/diamond 范围；各 target 断言其
  精确盈利子集及稳定拒绝，checked lane no-failure、strict f64、footprint 和 epilogue 正确。
- [ ] 同一 scalar pre-state 的 frontier 至多提交一个 winner；residual SLP 不进入 committed loop
  region；stable order/stats/explanations 不受存储顺序影响。

## E. 差分、结构和机器码

- [ ] Fixed-seed O0/O3 differential 覆盖 zero/short/exact/remainder/max-safe/overlap/disjoint/aligned/
  misaligned、四 safety mode、checked failure 和所有三后端适用路径。
- [ ] Adversarial irreducible/dependence/effect/strict-f64 reduction/possible-first-error/address-overflow/
  over-budget case 全部保持 scalar。
- [ ] 每个 accepted vector kernel 同时有 optimized KIR vector、pre-LLVM vector 和 pinned object
  SIMD disassembly 证据；不能由 LLVM 自发 vectorization 代签。
- [ ] baseline/native correctness 与 feature containment 在六 host 通过。
- [ ] x86-64 MSVC f64 Native DLL 在 `/nodefaultlib` 下由生成 object 的 coalescible `_fltused`
  闭包成功链接；与 runtime 副本共同链接无 duplicate、无新 export 或 Runtime ABI 漂移。

## F. 性能、尺寸与编译耗时

- [ ] Schema 7 与 rotating-twelve-channel-v1 在 x86-64/AArch64 各有完整报告；实际 0.11/0.10
  replay、current/replay Clang、checked/unchecked streams 都存在。
- [ ] C/Rust SIMD `interleaved-upper-median-three-channel-v2`：每 kernel >=90%，两架构各自
  geometric mean >=95%；每个保留行的 7 轮 raw call 均轮转 CK/C/Rust 三通道后取各自上中位数。
- [ ] 四元素 `slp_quad` 对每个保留行的三通道共同乘性因子作 geometric-mean 归一化，再对
  每通道执行未改变的 16/20、75%..125% 稳定性门槛；性能比较仍只使用原始上中位数耗时。
- [ ] Domain-fact suite 相对更快 generic C/Rust oracle 在两架构 geometric mean 都 >5%。
- [ ] Unknown-trip Loop SIMD 的 proposer/checker 均执行 target-specific runtime admission floor：
  x86-64 至少 `ceil(4 / UF) * VF * UF`、AArch64 至少 `2 * VF * UF`；不得跨 target 套用错误
  control penalty。
- [ ] Scalar corpus 相对 actual 0.11 replay geometric mean slowdown <=3%，individual <=8%；所有
  既有 0.11/0.10/Clang/optimizer latency gate 保持。
- [ ] Native object size aggregate <=35%，individual <=2.5x；source-to-object compile time geometric
  mean ratio <=1.5，individual <=2，采样/顺序/upper median 完整。
- [ ] Oracle precondition manifest、differential、UB audit、artifact/source/recipe digests 完整；无
  缺失 competitor 或测后 exclusion。

## G. 本地质量与审计

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `cargo test --all-features --locked`
- [ ] `cargo build --release --features native-toolchain --locked`
- [ ] `scripts/test-sanitized-ownership.sh`
- [ ] release/native-artifact/JIT-memory audits 全通过。
- [ ] `git diff --check`，无 ignored tests、临时文件、benchmark output 或 LLVM prefix 被提交。

## H. exact-SHA 十作业 CI

- [ ] quality 校验 85 项 `tests/oracles/typescript/SOURCE_MANIFEST.sha256`，以 frozen、
  script-disabled lockfile 构建仓库内 TypeScript oracle，并实际通过 C/WASM/CLI/fixture live
  differential gates；无私有仓库读取、registry 替代或 oracle test skip。
- [ ] native integration 通过。
- [ ] darwin-arm64、darwin-x64、linux-arm64、linux-x64、win32-arm64、win32-x64 六 host 通过。
- [ ] x86-64 与 AArch64 performance 通过。
- [ ] Workflow run 的 head SHA 精确等于最终候选 SHA；required job 无 skip/cancel/
  continue-on-error 替代。

## I. 文档与交付

- [ ] README/current docs/changelog/CLI help/release policy 的英文与中文镜像和实现一致。
- [ ] 0.13 multiversion/PGO 与 0.14 Auto-Tuning 仍明确为 future，不误报已实现。
- [ ] 每个阶段在 ignored acceptance bundle/CI artifact 中记录对应 SHA、RED 证据、命令/count/
  toolchain；真实设计修订有复诊文档且未降低门槛。
- [ ] 最终提交完成后不再修改工作树，等待用户审查，不自动合并主分支。

## 最终执行记录

完成时把 candidate SHA、parent/main SHA、Rust/LLVM/Clang/host identities、default/all-feature/
release counts、performance report/checker digests、CI run id/URL/job summary、git status/worktree
状态写入已忽略的 `target/acceptance/v0.12/final/`、CI artifact 与最终用户交付，不回写本文件。
任何新提交都会使 exact-SHA 证据失效，必须重跑相应门禁。
