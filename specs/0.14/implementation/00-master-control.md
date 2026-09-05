# CK 0.14 离线自动调优实施总控

> **状态：有效，按阶段 12–19 继续执行。** 阶段 01–11 是当前仓库已经提交的 v0.14
> 离线自动调优基础，必须在最终 SHA 回归重验；阶段 12–19 是本轮通过对抗性审查后新增的
> 优化兑现与跨平台修复链。任何阶段都不能单独签署 v0.14 完成。

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` and execute this plan inline task-by-task. The
> user explicitly forbids subagent-driven implementation after planning.

**Goal:** 在不改变 CK 语言、Native ABI 1 或 Runtime ABI 2 的前提下，实现有界、可审计、可缓存、可精确重放的 host-native O3 离线自动调优。

**Architecture:** 新的 `src/tune/` 子系统拥有 schema、workload、候选搜索、runner、测量、缓存与发布事务；`src/optimizer/` 只暴露经过独立检查的有限 CK 优化选择，Native backend 只接受已验证计划并返回可重放的 object/link 身份。`src/cli/tune.rs` 负责显式工作流编排，普通命令不读取任何调优状态。

**Tech Stack:** Rust 2024、现有 KIR 3/optimizer transaction、LLVM 22.1.8/LLD、SHA-256、平台原生进程与文件系统 API、Python 3 性能证据检查器、GitHub Actions。

---

## 固定输入与基线审计

本计划实现以下已通过八轮对抗性审查的规范：

- `specs/0.14/offline-autotuning.md`
- `specs/0.14/zh-CN/offline-autotuning.md`
- `specs/0.14/decision-schema-1.md`
- `specs/0.14/inspection-schema-1.md`
- `specs/0.14/publication-journal-1.md`
- `specs/0.14/performance-schema-9.md`
- `specs/0.14/review/design-adversarial-review-08.md`
- `specs/0.14/predicated-update-performance-1.md`
- `specs/0.14/review/optimization-fulfillment-adversarial-review-06.md`
- `specs/0.14/review/implementation-blocker-10.md`
- `specs/0.14/implementation/implementation-design-correction-10.md`
- `specs/0.14/implementation/implementation-design-correction-11.md`

实施分支为 `design/v0.14-offline-autotuning`，独立 worktree 为
`.worktrees/v0.14-offline-autotuning-design`，通过审查并固化证据的起点为
`1f27df4b7992f1209f6762aeb11632509d888ae0`。v0.14 最初基于 v0.13 候选
`94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`；最终接纳的 v0.13 修订为
`f82baf42b762e9b19542bcb0af593c1de9252891`。两者之间的累计提交已按
`implementation-design-correction-10.md` 逐文件复核并以等价或更严格的 v0.14 实现吸收；
历史 replay 也固定到该最终 SHA，移动分支、tag 或旧 CI 不得替代此身份。

目标是在本 worktree 形成完整、可审查的 0.14.0 候选并提交。不得自动合并 `main`，不得
创建或移动 tag，不得创建 GitHub Release。只有在本地门禁完成后才能推送 feature branch 触发
exact-SHA CI；长任务只做间隔查询，不在前台持续等待。

## 不可变执行规则

1. 本计划提交后完全行内执行，不再创建、恢复或使用任何子代理。
2. 阶段 01–11 已形成实现基础；严格按 12–19 顺序执行，且由阶段 19 在最终 SHA 回归 01–11
   的本地验收。每个新增行为先写最小 RED 并确认失败来自缺失能力，再实现 GREEN；阶段 task
   和 acceptance 全部完成后才进入下一阶段。
3. 调优始终显式 opt-in。普通 `check/run/build/build-llvm/emit-*` 不运行 harness、不读写
   `tune-v1`、不读取 `.cktune`，且普通 O3 收益阈值不变。
4. 测量只证明收益；安全、合法性、guard、first-error、effect order、strict `f64`、目标特性与 ABI
   始终由静态 checker 证明。trial typestate 永远不能直接进入生产输出或普通 object cache。
5. 搜索必须完整执行冻结的零基 expansion、beam/diversity、compile selection、size finalist、
   smoke/search/双 validation 状态机；任何省略、插入、预算缩短或样本拼接都 fail-closed。
6. `.cktune`、manifest/input map、inspection、journal、cache 和 schema 9 均采用冻结的 bounds、
   tag、排序、digest 与 exact EOF；未知或非 canonical 输入不得被宽松接受。
7. runner 是用户显式授权的任意代码，不宣称 hostile sandbox。实现必须提供规范要求的 cooperative
   process group/Job Object containment、完整 timeout、输出上限、fresh input staging 和空环境。
8. 发布必须使用完整 overlap closure、persistent destination locks、journal generation、primary-last
   barriers 与穷举恢复；旧的 best-effort `OutputTransaction` 不能代替 tune output-set protocol。
9. 不得为通过功能、性能、尺寸、编译时间或 CI 而降低阈值、减少 corpus、允许 selective rerun、
   放宽身份或把 required job 改为 optional。真实规范反例须先写 implementation blocker 复诊，
   同步修订英中规范、总控、相关 task/acceptance 和测试。
10. 生成物只进入已忽略的 `target/`、`build/`、cache 或私有临时目录。不得提交 profile、decision
    运行产物、性能 report、CI artifact、动态 run id 或本地 secret。
11. 每阶段在 `target/acceptance/v0.14/stage-NN/` 记录被测 SHA、RED 摘要、命令、测试计数、
    Rust/LLVM/Clang/host identity。旧阶段日志不能代替最终 SHA 的总验收。
12. 实施期成立的真实设计复诊以 `implementation-design-correction-01.md` 至 `11.md` 为完整序列；
    其中修订只能闭合可实现性与证据真实性，不得降低本总控或规范门槛。
13. 阶段 12–19 不重写 CKTUNE01、Manifest Schema 1、KIR 3、Native ABI 1 或 Runtime ABI 2。
    predicated-update 继续复用 Loop SIMD payload；独立 gate 通过 source-aware 重建证明唯一 choice
    和动态可达，不新增可伪造的 wire 布尔字段。

## 冻结实现架构

```text
explicit CLI + closed manifest -> no-follow runner/input snapshots
  -> ordinary verified O3 pre-tune KIR + exact target/profile/mode identity
  -> finite CK-owned sites/units/variants -> closed beam search trace
  -> non-publishable Native trials -> size filter -> runner smoke/search
  -> two validation rounds -> certificate or measured baseline reason
  -> independent plan replay + object graph/link recipe verification
  -> CKTUNE01 decision + CKCOBJ04/cache schema 5
  -> overlap-closed journaled decision/sidecar/primary publication
  -> source-aware single-choice predicated-update attestation
  -> independent PGO-only versus PGO+tuned Floyd evidence contract
```

`src/tune/` 是 compiler-owned 子系统，并与 CK workload profile 的 `src/profile/`、target cost
profile 的 `src/ir/kir/profile.rs` 分离。`src/optimizer/tune.rs` 只负责从 canonical pre-tune KIR
枚举和重放有限选择；`src/tune/` 负责收益搜索。`src/backend/artifact/` 提供代码产物及闭合 link
recipe 身份，不接受调优策略。`src/cli/tune.rs` 是薄编排层。

## 阶段顺序

| 阶段 | 交付物 | 主要仓库落点 | 前置 |
| --- | --- | --- | --- |
| 01 | CKTUNE01 schema、bounded codec、self-contained checker、JSON/text inspect 与 golden fixtures | `src/tune/{schema,decision,inspect}.rs`, `tests/tune/` | 无 |
| 02 | closed manifest、路径/环境、runner/input immutable snapshot 与 CKTIMAP1 | `src/tune/{manifest,path,snapshot,input_map}.rs` | 01 |
| 03 | stable site/unit/variant、完整 expansion trace、beam/diversity 与 exact plan replay | `src/optimizer/tune.rs`, `src/tune/{frontier,search,plan}.rs` | 01 |
| 04 | non-publishable trial、Native object/link/size 身份与 source-aware replay checker | `src/tune/{trial,replay}.rs`, Native backend | 03 |
| 05 | runner process protocol、cooperative containment、timer、calibration 与 timeout typestate | `src/tune/runner/`, platform tests | 02,04 |
| 06 | smoke/search/双 validation 调度、Q32/stability/selection/certificate 与完整 decision assembly | `src/tune/{measure,selection,session}.rs` | 01–05 |
| 07 | canonical destinations、overlap locks、CKTJNL01、barriers、recovery 与 primary-last publication | `src/tune/publication/` | 01,04 |
| 08 | compile/measurement/completed-decision cache、CLI tune build/inspect/tune-use 与普通路径隔离 | `src/tune/cache/`, `src/cli/tune.rs`, `src/cli/*` | 01–07 |
| 09 | 0.14 identity、CKCOBJ04/schema 5、兼容矩阵、双语 current docs 与 release audit | Cargo/docs/contracts | 08 |
| 10 | schema 9 corpus、runner/oracles、collector、checker、archive 与本地 performance contract | benches/scripts/performance tests | 09 |
| 11 | exact-SHA 十作业 CI、六 host/两 performance gate、最终本地与远程验收 | workflow/contracts | 10 |
| 12 | Windows/Unix profile runtime 原子与持久 shard 发布修复 | `native/profile_runtime/`, profile tests | 01–11 已落代码 |
| 13 | host artifact 路径与 LLVM void-call Native 回归修复 | bridge/CLI/native tests | 12 |
| 14 | predicated same-place update 发现、Memory SSA 与合法性模型 | vector analysis/tests | 13 |
| 15 | compare/select/unmasked-store 物化、独立 checker 与 LLVM 结果 | vector pass/checker/native tests | 14 |
| 16 | Loop SIMD 调优候选保留、唯一 choice 与 source-aware attestation | optimizer/tune/CLI tests | 15 |
| 17 | 冻结 Floyd source/input/manifest 与四协议 native runner | benches/tune/performance tests | 16 |
| 18 | Contract 1 collector、closed report、checker 与 mutation tests | benches/scripts/performance tests | 17 |
| 19 | 十作业 CI 接线、全量本地复验、exact-SHA 远程门禁与交付 | workflow/contracts/final evidence | 18 |

每个阶段都有同号 `*-task.md` 与 `*-acceptance.md`。当前
`99-final-acceptance.md` 是唯一总验收清单；
阶段通过不能代签 source-aware replay、六平台、性能、exact-SHA CI 或 v0.13 accepted-base 门禁。

## 提交与执行策略

- 本总控、全部阶段任务、阶段验收、总验收和自审先形成一个独立计划提交。
- 实现提交使用 `compiler(stage-NN): <imperative outcome>`；测试和 fixture 可与对应阶段一起提交，
  但每阶段至少留下一个可单独回退的清晰检查点。
- 计划执行使用 `superpowers:executing-plans`；实现行为遵循
  `superpowers:test-driven-development`，失败诊断遵循 `superpowers:systematic-debugging`，阶段及最终
  完成声明遵循 `superpowers:verification-before-completion`。
- 性能 report 先由 collector 写原始证据，再由独立 checker 判定；collector、benchmark 或人工观察
  均无权宣布门禁通过。
- exact-SHA CI 运行期间若本地提交任何影响实现、fixture、checker、规范或计划的变更，旧 run 作废，
  对新 SHA 重新执行受影响的门禁。

## 阻断处理

- 实现缺陷：保留最小 RED，修复实现，不修改规范门槛。
- 测试缺陷：仅当测试与冻结规范矛盾或无法观察指定行为时修改，并在阶段证据记录具体反例。
- 环境缺陷：修复或记录 host/toolchain 能力；Native/performance/CI required gate 不得改成 skip。
- 规范缺陷：先新增 `specs/0.14/review/implementation-blocker-NN.md` 复诊；成立后同步修改双语规范、
  normative attachment、总控、受影响 task/acceptance 和测试，再继续。
- 远程缺陷：区分产品失败、runner/capability 失败与暂态基础设施失败；不得以本地或旧 SHA 结果代替。

Exact V0.13 run `33795954634` 暴露的 Linux 跨 vCPU measurement band 与 schema8 累计证据目录
缺失，按 `specs/0.14/review/implementation-blocker-17.md` 累计闭环：V0.14 继承 same-core case scope、
self-contained schema8 evidence 与回归，并重钉 exact V0.12/V0.13 replay。门槛、样本、统计、corpus、
tuning policy、语言/ABI 与十作业拓扑均未改变。

Exact V0.14 run `33808562098` 进一步证明 same-core affinity 不能排除 hosted
runner 对短测量的 deschedule/throttling 污染。按
`specs/0.14/review/implementation-blocker-18.md`，继承的 Linux schema-7 runtime
sample 改用当前线程 CPU time，同时保留原 affinity 与 32 轮 conditioning；失败的 historical
schema-8 report/evidence 在 checker 前复制到非隐藏 artifact 目录。V0.13 replay 已重钉到
`6dba7ada778dab868a8e7c507db9c2c0d85c9749`。门槛、样本、统计、timed work、corpus、
tuning policy、语言/ABI 与十作业拓扑均未改变。

后续 exact v0.12/v0.13 CI 证明两个 accepted revisions 仍分别存在跨目标短循环摊销与 PGO
初始化热路径问题。V0.14 已逐差异继承修复后的 v0.12
`0de952ba5f17ad353ffb00f59b6349c96568b239` 和 v0.13
`6dba7ada778dab868a8e7c507db9c2c0d85c9749`，并重钉两个 replay manifest。复诊见
`specs/0.14/review/implementation-blocker-19.md`；所有性能门槛、timed work、样本、统计、
corpus、稳定性规则与 required job topology 均保持不变。

Exact v0.13 run `33820321093` 的 AArch64 performance job 随后暴露 schema-7 的 32-batch
conditioning 被错误放入七次原始计时调用，实际每个保留 sample 执行 224 batch；本地复验还
暴露旧 v0.13 的 Darwin `fstat` SDK 降低符号未闭合。V0.14 已逐差异继承修复后的 exact v0.12
`0de952ba5f17ad353ffb00f59b6349c96568b239` 与 v0.13
`6dba7ada778dab868a8e7c507db9c2c0d85c9749`，同时保留 v0.14 已有、更严格的直接
`fgetattrlist` runtime 实现。复诊见 `specs/0.14/review/implementation-blocker-20.md`；所有性能
门槛、timed work、样本、统计、corpus、稳定性规则与 required job topology 均保持不变。

Exact v0.12 run `33823603857` 又证明单轮 32-batch settling margin 仍让 Rust `slp_quad`
只有 15/20 个样本落在稳定带内。v0.14 已继承固定 64-batch、once-per-retained-sample
ramp，并把 accepted v0.13 与 replay 重钉到
`2baa45a49c687692dc3cba05a627742cbfdcbe69`。复诊见
`specs/0.14/review/implementation-blocker-21.md`；所有 timed work、样本、统计、性能与稳定性
门槛、corpus 及 required job topology 均保持不变。

Exact v0.12 run `33825887411` 随后证明任何固定 ramp 长度都不能可靠选择托管 AArch64
`slp_quad` 的约 4.42/8.84 ms 频带。v0.14 已继承失败关闭的
`bounded-upper-band-v1`，并把 accepted v0.13 与 replay 重钉到
`f82baf42b762e9b19542bcb0af593c1de9252891`。复诊见
`specs/0.14/review/implementation-blocker-22.md`；所有 timed work、样本、统计、性能与稳定性
门槛、corpus 及 required job topology 均保持不变。
