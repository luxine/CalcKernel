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
- `specs/0.14/review/implementation-blocker-25.md`
- `specs/0.14/review/implementation-blocker-30.md`
- `specs/0.14/review/implementation-blocker-31.md`
- `specs/0.14/review/implementation-blocker-32.md`
- `specs/0.14/review/implementation-blocker-33.md`

实施分支为 `design/v0.14-offline-autotuning`，独立 worktree 为
`.worktrees/v0.14-offline-autotuning-design`，通过审查并固化证据的起点为
`1f27df4b7992f1209f6762aeb11632509d888ae0`。v0.14 最初基于 v0.13 候选
`94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`；最终接纳的 v0.13 修订为
`5c6220758718b1ceac8ae32aec80c660d7b67b5e`。两者之间的累计提交已按
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

Exact v0.12 run `33833225186` 证明绝对目标频带方案不成立，v0.14 因而继承
`interleaved-upper-median-three-channel-v2`。Exact v0.12 run `33966418774` 又证明 x86
`VF4/UF2` noalias kernel 的逐 chunk load/compute/store 顺序隐藏了已证明的并行。v0.14 已继承
x86 `UF > 1` 的 SSA/MemorySSA 就绪列表调度，并把 accepted v0.13 与 replay 重钉到
`4472330758a71312f9a86b83ab10fdce47791287`。复诊见
`specs/0.14/review/implementation-blocker-23.md`；所有 timed work、样本、统计、性能与稳定性
门槛、corpus 及 required job topology 均保持不变。

Exact v0.14 run `34014114894` 的 AArch64 performance job `101435039015` 证明，旧
accepted v0.13 在 128-bit SVE 上未形成超过 Advanced-SIMD baseline 的循环级并行度，
schema-8 eligible suite 的 dispatch geo improvement 只有约 1.003。V0.14 已逐差异继承
v0.13 的 AArch64 SVE 四路 LLVM interleave 修复，并把 accepted v0.13 与 replay 重钉到
`4472330758a71312f9a86b83ab10fdce47791287`。复诊见
`specs/0.14/review/implementation-blocker-24.md`；所有 timed work、样本、统计、性能与稳定性
门槛、corpus、target tiers 及 required job topology 均保持不变。

Exact v0.13 run `34017771182` 的 x86-64 performance job `101444674413` 随后证明，
candidate-constant profiling 的逐元素原子调用会令 `branch-layout` generation 开销达到 ordinary
的 5.68 倍，超过不变的 5.0 上限。Exact v0.14 run `34017772543` 的 AArch64 performance
job `101444700041` 同时证明，generic SVE 的默认调度在 `compute-bound` 上比同一已选 target 的
direct 调用慢约 5.52%，超过不变的 5% 上限。V0.14 已继承 candidate counter 函数内批处理和
固定 `neoverse-n2` 调度模型（不扩展 generic SVE/SVE2 ISA），并把 accepted v0.13 与 replay
重钉到 `b159ea7588359116bb94396215ff793b69e77235`。复诊见
`specs/0.14/review/implementation-blocker-25.md`；所有 timed work、样本、统计、性能与稳定性
门槛、corpus、target tiers 及 required job topology 均保持不变。

Exact v0.13 run `34021087906` 的 x86-64 performance job `101453829767` 随后证明，
`specialized_length` 的 Native handoff 只形成四向 XMM 调度，未达到相同语义下两份五向 SIMD
oracle 的不变 90% 门槛；同 run 的 AArch64 performance job `101453829694` 还证明，独立
multiversion module 的 KIR 复验丢失 verified contract facts，令 `compute-bound` SVE member
重新引入 alias-versioning，并以约 5.51% 超过不变的 5% direct-dispatch 上限。Exact v0.14
run `34021089423` 的 replay job `101453852634` 复现后者。V0.14 已逐差异继承两项修复，并把
accepted v0.13 与 replay 重钉到 `966d54b075a76f2f493d51cb0764688c2ca85675`，manifest SHA-256
为 `d29ecfde60ef72eb46f51016d9e67d8cd1606bbc206e1a20580dd7cbaf235c62`。复诊见
`specs/0.14/review/implementation-blocker-26.md`；所有 timed work、样本、统计、性能与稳定性
门槛、corpus、target CPU/features、target tiers 及 required job topology 均保持不变。

Exact v0.13 run `34028252202` 的 AArch64 Linux native-host job `101473242935`
与 exact v0.14 run `34028600132` 的 native integration job `101474136852`、x86-64 Linux
native-host job `101474136938` 随后共同证明，dispatcher fact ledger 会错误重复登记仅存在于
baseline root 函数体内的 assume/range/no-wrap/alias-scope 证据。V0.14 已继承只复制真实
dispatcher 参数/函数属性的修复，并把 accepted v0.13 与 replay 重钉到
`aa155825959e49d61fcea7a953935b179a7a238f`，manifest SHA-256 为
`d8a0b50eba1957c980b4a6acfed0e2e2e481b9cd804cb8e1b44ae7c7b14129f5`。复诊见
`specs/0.14/review/implementation-blocker-27.md`；fact-audit equality、语言/ABI、target、性能与
稳定性门槛、timed work、样本、corpus 及 required job topology 均保持不变。

Exact v0.14 run `34031421321` 的 x86-64/AArch64 performance jobs
`101481682943`/`101481682918` 在准备 exact v0.13 replay 时又证明，x86 target profile 未向
checked KIR cost model 暴露已在封闭 frontier 内的四路 vector chain，而 AArch64 generic SVE
函数只带 `tune-cpu` 时没有形成实际使用该调度模型的 per-function subtarget。V0.14 已继承
exact v0.13 `bd0210b4ce89c5001f46ab2128a8c7a73dc6323a` 的两项修复，并把 accepted v0.13
与 replay 重钉到该 SHA，manifest SHA-256 为
`925eb2410310f4e3aa31247be37f3576468a12cb2a54a421a64f7ed651304d6d`。复诊见
`specs/0.14/review/implementation-blocker-28.md`；语言/ABI、target ISA、tuning search、性能与
稳定性门槛、timed work、样本、corpus 及 required job topology 均保持不变。

Exact v0.14 run `34034844096` 的 v0.13 replay 随后证明，full-root budget 只保留 v4 时 required
v3 worker 会退回 baseline，而旧 `selectedDirect` 实际是 `--cpu native` 近似物；同 run 的 x86
native/Clippy 还暴露 exact `UF4` 代理断言与 target-specific test import 问题。V0.14 已继承 exact
v0.13 `4a04fb34eb0f1358d0f8fa308f95d031954e72b0` 的完整闭环，并把 accepted v0.13 与 replay
重钉到该 SHA，manifest SHA-256 为
`b713147e369c2c4ebd5debcf61005531e5154961ddf864f14bede8baa8599eca`。复诊见
`specs/0.14/review/implementation-blocker-29.md`；语言/ABI、target ISA、profitability floor、性能
与稳定性门槛、timed work、样本、corpus 及 required job topology 均保持不变。

Exact v0.14 run `34038295553` 的独立 v0.13 replay 证明上一轮 coverage-first 修复仍发生在
isolated profitability filter 之后，因此 x86 required v3 worker 看不到仅物化的 v4 member；同一
run 还证明 Linux AArch64 dynamic library 没有 CK startup entry capture，resolver 因 auxv 不可用
合法退回 baseline。V0.14 已逐字继承 exact v0.13
`d2a2e5f9fb7ed0c71b1ded4d3bf8c789b4d774ac` 的 compatibility-companion retained-set 与
freestanding binary `/proc/self/auxv` fallback，并把 accepted v0.13 与 replay 重钉到该 SHA；manifest
SHA-256 为 `b730be3b2d40efd7f35195c54e2472b107ccf87f7e85f3095422609f328ea2c0`。复诊见
`specs/0.14/review/implementation-blocker-30.md`；语言/公开 ABI、strict FP、安全规则、target ISA、
性能与稳定性门槛、timed work、样本、corpus、平台及 required job topology 均保持不变。

Exact v0.14 run `34041903921` 的 AArch64 performance job `101510076457` 在准备历史
schema-8 replay 时精确复现 v0.13 run `34041456107` 的两个真实缺口：x86 三流 noalias map 的
`VF4/UF4` 物化因冗余 stride/MemorySSA 表示越过既有 `2x` KIR growth 上限；AArch64 128-bit
SVE2 上，无 profile multiversion 将中等冷分支 helper 克隆并重新 inline，dispatch geo 只有
`1.01545 < 1.08`。V0.14 已逐字继承 exact v0.13
`b7da701ff7785c4a687ebfd65a8882b1b2a4eac2` 的 compact UF/MemorySSA 表示、独立 checker、
8-instruction multiversion inline budget 与 Native `noinline` closure，并把 accepted v0.13 与 replay
重钉到该 SHA；manifest SHA-256 为
`2bb3430ce53239ab13b91baffb84c3337e2fbe9d9c3b48a34d59f41bb603a8ac`。复诊见
`specs/0.14/review/implementation-blocker-31.md`；语言/公开 ABI、strict FP、安全规则、target ISA、
growth/profitability/性能与稳定性门槛、timed work、样本、corpus、平台及 required job topology
均保持不变。

Exact v0.13 run `34049750799` 的 AArch64 performance job `101531090175` 随后通过
累计 schema 7 与全部 schema 8 runtime gate，但 branch-layout multiversion 动态库以
`4576 / 1808 = 2.53097 > 2.5` 未通过单项 artifact-size gate。V0.14 已逐字继承 exact v0.13
`2ba127c18a6c5f831dc55814c37da2dd1ecedea6` 的 shared-link dead-section closure：
Mach-O/COFF/ELF 分别使用 `-dead_strip`、`/opt:ref`、`--gc-sections`，并把 accepted v0.13
与 replay 重钉到该 SHA；manifest SHA-256 为
`e4c8fceab818681350f52715ee334ccfe7325a99167261350c0e2269843e7873`。复诊见
`specs/0.14/review/implementation-blocker-32.md`；语言/公开 ABI、strict FP、安全规则、target ISA、
tuning/growth/profitability/性能与稳定性门槛、timed work、样本、corpus、平台及 required job
topology 均保持不变。

Exact v0.13 run `34051103711` 的 AArch64 performance job `101534772807` 在前一轮
单项尺寸修复后通过全部 runtime gate，但五个 multiversion 动态库总尺寸仍为
`20584 / 9632 = 2.13704 > 2.0`；exact v0.14 run `34051523126` 的 job
`101535896793` 在准备同一历史 replay 时精确复现该阻断。V0.14 已逐字继承 exact v0.13
`5c6220758718b1ceac8ae32aec80c660d7b67b5e` 的 closure：ELF shared link 使用
`--strip-all`，公开入口由 `.dynsym` 解析，唯一私有 resolver slot 位于 pointer-width/aligned
`.ck_dispatch_slot` section，允许 LLD 的 `SHT_PROGBITS` 或 `SHT_NOBITS` allocated writable
表示；生成的 per-root acquire/release slot 是唯一 publication
layer，one-shot detector 删除重复的 process cache 并按 size-first recipe 编译。重建失败
artifact 的同形链接得到 aggregate `14792 / 8544 = 1.73127`，低于未改变的 `2.0` 门槛；
accepted v0.13 与 replay 重钉到该 SHA，manifest SHA-256 为
`c6f5242e68907ba511777ab104bb66d0c2126eafd53be8a21e3e4257ac00b86f`。复诊见
`specs/0.14/review/implementation-blocker-33.md`；语言/公开 ABI、strict FP、安全规则、
target ISA、tuning/growth/profitability/性能与稳定性门槛、timed work、样本、corpus、平台及
required job topology 均保持不变。

Exact v0.13 run `34077713073` 的 x86-64 performance job `101607114352` 随后以
稳定的两路 XMM `map_u32` schedule 未通过未改变的 SIMD oracle 门槛；exact v0.14 run
`34077978310` 的 x86-64 replay 复现该失败，而 AArch64 job `101607713997` 在历史性能
checker 通过后把 Cargo 合法重建的 build-tree compiler 误判为冻结 compiler 变化。V0.14
已逐字继承 v0.13 `5c6220758718b1ceac8ae32aec80c660d7b67b5e` 的 compact standalone
UF4 closure，并重钉 replay manifest SHA-256
`c6f5242e68907ba511777ab104bb66d0c2126eafd53be8a21e3e4257ac00b86f`。历史 replay
只执行和复核 owned replay 目录中的 immutable compiler copy；Cargo build-tree output 不再是
identity input。复诊见 `specs/0.14/review/implementation-blocker-34.md`；语言/公开 ABI、
strict FP、安全规则、target ISA、growth/profitability/性能与稳定性门槛、timed work、样本、
corpus、平台及 required job topology 均保持不变。
