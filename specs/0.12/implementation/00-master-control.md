# CK 0.12 事实驱动向量优化器实施总控

## 固定输入与目标

本计划实现已经通过第五轮对抗性审查的双语规范：

- `specs/0.12/fact-driven-vector-optimizer.md`
- `specs/0.12/zh-CN/fact-driven-vector-optimizer.md`

审查链位于 `specs/0.12/review/`。实施分支是
`feature/v0.12-vector-optimizer`，独立 worktree 是
`.worktrees/v0.12-vector-optimizer`，起点为
`6c00b8044fe2e9179726fc2730951403822e7468`。0.11 性能 replay 固定为
`80c0acf6bb5d65e4d9d40352b9501ea32b79f43d`，不得用移动分支替代。

目标是在该分支形成完整、可审查的 0.12.0 候选并提交。不得自动合并 `main`，不得创建
tag 或 GitHub Release。为满足 exact-SHA 六主机/性能门禁，可以推送 feature branch 并
显式 `workflow_dispatch` CI；这不授权创建 PR 或合并。远程长任务只定期查询状态，不在
前台原地等待。

## 不可变执行规则

1. 完全行内执行，不使用子代理。
2. 严格 TDD：每项行为先增加最小测试并实际观察与缺失实现一致的失败，再写最小生产
   代码使其通过，最后重构。编译错误可以是 schema migration 的 RED，但必须确认错误点
   正是新增契约。
3. 阶段必须按顺序执行；每阶段 task 全部完成且 acceptance 全部通过后才能进入下一阶段。
4. 不改变 0.11 的 source syntax、semantic MIR、诊断、strict-f64、checked first-error、
   print/runtime 顺序、slice ABI、public Native ABI 1 或 Runtime ABI 2。
5. 不允许 proposer 自证。所有新变换必须由不调用 proposer/analysis implementation 的
   checker 基于 pre-state、closed record、facts/proofs/profile 独立复算。
6. Normal rejection、reuse、非获胜 candidate 与预算耗尽都保留 scalar correctness，并永久
   计入 audit ledger；false certificate 或 post-commit verifier failure 是 compiler error。
7. 不为通过功能、性能、size、compile-time 或 CI 而降低门槛、删语料、忽略测试或改统计。
   只有真实规范反例可先复诊，再同步修改双语规范、master、相关 task/acceptance 与测试。
8. 生成物只进入已忽略的 `target/`、`build/` 或显式临时目录；不提交 LLVM prefix、benchmark
   输出、CI artifact 或本地 agent 文件。
9. 每个阶段开始前确认 worktree/branch；每个阶段完成后记录 SHA、命令、测试计数、toolchain
   identity 和 RED 证据摘要。禁止用旧日志替代当前 SHA。

## 冻结架构

```text
CheckedProgram -> scalar semantic MIR
  -> consumer/mode-specific scalar KIR v2 + KirTargetProfile schema 1
  -> verified O1 prefix
  -> scalar-only specialization transaction
  -> existing O2 pipeline
  -> canonical loop/dependence/cost pre-state
  -> independently checked Loop SIMD | unroll(+SLP) frontier
  -> residual SLP -> final verifier
  -> C scalar | WASM scalar | audited Native LLVM fixed vectors
```

Speculative state严格分层：`KirVerifiedProgramState` 可以整体回滚；
`KirOptimizationAuditState` 只能单调扣预算与追加记录。KIR profile、proof、cost、budget 与
cache identity 的 schema/version 常量必须集中定义并进入 mutation tests。

## 阶段顺序

| 阶段 | 交付物 | 主要仓库落点 | 前置 |
| --- | --- | --- | --- |
| 01 | KIR v2/profile 外壳、pipeline 参数、精确 consumer CLI | `src/ir/kir`, `src/optimizer/kir_pipeline.rs`, `src/cli` | 无 |
| 02 | Vector/Mask KIR 指令、printer、structural verifier、proof record schema | `src/ir/kir`, `src/optimizer/{proof,verify}.rs` | 01 |
| 03 | LLVM TTI profile bridge、Native vector lowering、fact audit、bridge ABI 3 | `native/bridge`, `src/backend/llvm`, `tests/native` | 02 |
| 04 | loop-simplify/LCSSA、affine access/dependence、total version predicate | `src/optimizer/analysis`, `src/optimizer/kir_passes` | 03 |
| 05 | verified/audit state 分层、transaction、预算 ledger、独立 plan checker 与 O3 frontier 骨架 | `src/optimizer/{kir_pipeline,verify,proof}.rs` | 04 |
| 06 | fact-driven direct-call specialization | `src/optimizer/analysis`, `src/optimizer/kir_passes` | 05 |
| 07 | controlled full/partial unroll 与 SLP proposal/checker/materializer | `src/optimizer/kir_passes`, `src/optimizer/analysis` | 06 |
| 08 | Loop SIMD/versioning/reduction、frontier winner、端到端 differential/explanations | optimizer + three backends + CLI tests | 07 |
| 09 | 0.12.0 candidate identity、current docs、compatibility/release contracts | Cargo/docs/contracts/release workflows | 08 |
| 10 | performance schema 7、0.11 replay、C/Rust SIMD oracle、size/compile-time 与 exact-SHA CI | `benches`、`scripts`、`.github/workflows` | 09 |

阶段 01–10 各有同号 `*-task.md` 与 `*-acceptance.md`。`99-final-acceptance.md` 是唯一总验收
清单，阶段通过不能代签后续性能或 exact-SHA CI。

## 提交策略

计划与审查文档先单独提交。实现阶段提交信息使用：

    optimizer(stage-NN): <imperative outcome>

阶段内可有 TDD 小提交，但每阶段至少形成一个清晰检查点。动态执行证据写入已忽略的
`target/acceptance/v0.12/stage-NN/`，远程证据保存在 CI artifact；不得把 run id、最终 SHA
或动态勾选回写进被测提交。这样最终 exact-SHA 稳定且无自引用。若必须修订仓库内规范或
实现，则产生新 SHA 并重跑受影响门禁。最终工作区必须干净，`main` 仍由用户控制，分支只
等待审查。

## 阻断处理

- 实现缺陷：保留 RED，修复实现。
- 测试缺陷：只有测试违背冻结规范时修改，并记录反例。
- 环境缺陷：记录 host/Rust/LLVM/Clang identity，修复环境；Native/CI gate 不得改为 skip。
- 规范反例：先在 `specs/0.12/review/implementation-blockers-*.md` 复诊，成立后同步修订。
- 远程 CI：提交/推送后记录 run id 与 exact SHA，30–60 秒以上间隔查询；不持续占用前台等待。

阶段 10 在 GitHub 仓库从 `Rust_CalcKernel` 更名为 `CalcKernel` 后暴露了 TypeScript oracle
旧仓库名碰撞：quality 的第二次 checkout 会在当前 Rust 仓库中解析退役 TypeScript commit，因而
不能形成有效的 exact-SHA 验收。复诊与闭环见
`specs/0.12/review/implementation-blocker-03.md`。修复仅把固定提交的最小 oracle 源码与 fixtures
固化到 `tests/oracles/typescript`，保留 frozen lockfile、provenance 与 85 项 source manifest，并
继续执行原有 live differential gate；语言/ABI、性能阈值、corpus、schema 7 与十作业要求均不变。

候选 `8b6feb0dbe7e5df7d8e48d7398a23b9a8285d7ba` 的首次十作业 CI 又暴露了跨平台与性能
阻断：Windows differential cleanup 保留已加载 DLL；checked branch 未标记冷失败路径；x86
Rust integer-cast oracle 只实现有符号 `i32` 域；x86 KIR horizontal reduction 在每个 chunk
过早 fold；AArch64 超短 SLP 的 CK/C/Rust 三方样本共同出现频率状态双峰。复诊和闭环见
`specs/0.12/review/implementation-blocker-04.md`。本轮不降低 90%/95% 门槛，不改变 timed
corpus、样本数、统计量或 required job topology；只修复资源生命周期、完整 `u32` oracle
语义、target-specific lowering、冷路径 metadata，并为三个 channel 增加相同的 unmeasured
short-kernel conditioning。

候选 `d83805075b0ac8986c895b7a287c84eac509b7f9` 的 AArch64 performance job 又证明单次
conditioning 不足以让四元素 `slp_quad` 在计时前稳定进入持续执行状态：CK、C、Rust 三方都
出现约 `4.42 ms`/`8.84 ms` 的同源双峰，Rust 样本恰好跨过稳定性门禁。复诊和闭环见
`specs/0.12/review/implementation-blocker-10.md`。本轮把三个 channel 的计时前 conditioning
固定为四个相同的 unmeasured batch；timed batch、样本数、轮换顺序、统计量、阈值、corpus 与
平台矩阵均保持不变。

后续 v0.13 exact AArch64 replay 证明四批 conditioning 后三方仍可能共同落入相同的两档
执行状态，因此 v0.12 的一次通过不足以证明噪声源已经消失。复诊与闭环见
`specs/0.12/review/implementation-blocker-11.md`。Linux release-performance 测量现在先从
继承的 allowed affinity set 选择一个 CPU，并在整个三 channel case 期间固定线程，之后恢复
原 affinity；现有四批 conditioning 因而稳定作用于同一 core。所有 timed work 与门槛不变。

本机当前未设置 `CKC_LLVM_PREFIX`。阶段 01–02 可以先完成 default-feature TDD；阶段 03 前
必须按 README 使用固定 LLVM 22.1.8 release prefix，阶段 10 还需要 oracle profile 的 pinned
Clang 22.1.8 与 Rust 1.90.0。
