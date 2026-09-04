# 阶段 10 任务：schema 7 性能、replay/oracle、size/compile-time 与最终 CI

## 目标

在不修改冻结门槛的前提下实现严格性能报告 schema 7、独立 0.11 replay、手写 C/Rust SIMD
oracle、domain-fact suite、artifact size/source-to-object compile-time gate，并把十作业 exact-SHA
门禁接入 CI。

## 仓库落点

- `benches/{ckc_perf.rs,runtime_replay.rs,summary-schema.md}` 与 fixtures/adapters。
- `scripts/{prepare-performance-replay.py,check-native-performance.py,diagnose-native-performance.sh}`。
- `tests/performance/**`、`tests/contracts/ci.rs`。
- `.github/workflows/ci.yml` 与 bootstrap action（仅真实需要时改 recipe/cache key）。

## TDD 顺序

1. 写 schema/checker RED：candidate 0.12.0、schema 7、完整 identity/digests、未知/缺失/额外
   field fail-closed；不得接受历史数值替代实际 sample。
2. 写 rotating-12 RED：固定 channel 顺序、3 warmup/20 sample、row rotation、7 calls/sample、
   upper median、同进程/同 input/batch；任一 stream fail 立即失败。
3. 扩展 replay preparer/loader RED：exact commit `80c0acf...`、真实 `ckc 0.11.0`、LLVM/target/
   recipe/source/adapter/artifact bytes+SHA；symlink/path escape/duplicate/mutation 拒绝。
4. 写 rotating-3 RED：candidate/C SIMD/Rust SIMD，checked/unchecked 分开；generic domain gate 用
   pinned generic C/Rust；precondition manifest/differential/UB audit 缺一失败；dynamic symbol 与
   typed signature 在 runner 构造时只解析一次，计时循环不得执行 symbol/string dispatch。
5. 写 vector/domain threshold RED：每 kernel >=90% faster oracle、geo >=95%，domain geo >5%；
   精确等号边界、一个 invalid competitor、geomean counterexample。
   若 domain-fact 短循环因显式 vector control 未摊销而失败，只能修复 proposer 与 independent
   checker 共同执行的 target-specific admission floor（x86-64 四 chunk、AArch64 两 chunk）；
   不得降低 domain gate 或减少 timed work。
6. 写 scalar regression RED：相对 actual v0.11 replay geo <=3%、individual <=8%，既有 0.10/
   Clang/optimizer limits 全保留。
7. 写 size/compile-time RED：同 source/mode baseline object，aggregate <=35%、individual <=2.5x；
   alternating 3 warmup/15 measured upper median，geo <=1.5、individual <=2；fresh path/no cache。
8. 写 x86-64/AArch64 corpus 与 C/Rust oracle，实现 map/zip、f64、integer transform、modular
   reduction、SLP、runtime noalias、specialization，memory/compute bound 都有；先 correctness/UB，
   后测时。
9. 若 optimizer latency gate 失败，只允许优化精确 structural cache、重复 analysis 与
   candidate-free transaction allocation；changed function、module-global identity、fact/proof/rewrite
   verification、deterministic budget debit 与 materialized certificate digest 均不得省略。前序
   loop descriptor 只可在 induction structure 未改变且 function identity 精确匹配时复用。
10. 更新 CI 为 quality、native-integration、六 native hosts、两 performance jobs；上传完整 evidence，
   不把 diagnostic job 替换 gate。失败或显式请求的同 worker diagnostic 只检查实际测量对象，
   必须同时覆盖 32 个 candidate/current/replay-Clang scalar 对象、8 个 v0.11 replay Native 对象
   与 8 个 v0.10 replay Native 对象；不得读取已由 schema 7 删除的 `runtimeReplay` 字段，也不得
   重新构建或重新计时。

## 执行策略

- 本地先跑 schema/unit/小 correctness，稳定 worker 才跑真实性能。
- Feature branch push 后用 `gh workflow run ci.yml --ref <branch>`；记录 run id/exact SHA。长 CI 每次
  间隔查询，不前台持续等待。
- 任何性能失败先诊断实现/测量/环境，不改门槛、统计、oracle 或 corpus。
