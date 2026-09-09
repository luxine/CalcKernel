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
- `specs/0.14/review/implementation-blocker-45.md`
- `specs/0.14/review/implementation-blocker-46.md`
- `specs/0.14/review/implementation-blocker-47.md`
- `specs/0.14/review/implementation-blocker-48.md`
- `specs/0.14/review/implementation-blocker-49.md`
- `specs/0.14/review/implementation-blocker-50.md`
- `specs/0.14/review/implementation-blocker-51.md`
- `specs/0.14/review/implementation-blocker-52.md`
- `specs/0.14/review/implementation-blocker-53.md`
- `specs/0.14/review/implementation-blocker-54.md`
- `specs/0.14/review/implementation-blocker-55.md`
- `specs/0.14/review/implementation-blocker-56.md`
- `specs/0.14/review/implementation-blocker-57.md`
- `specs/0.14/review/implementation-blocker-58.md`
- `specs/0.14/review/implementation-blocker-59.md`
- `specs/0.14/review/implementation-blocker-60.md`
- `specs/0.14/review/implementation-blocker-61.md`
- `specs/0.14/review/implementation-blocker-62.md`
- `specs/0.14/review/implementation-blocker-63.md`
- `specs/0.14/review/implementation-blocker-64.md`
- `specs/0.14/review/implementation-blocker-65.md`
- `specs/0.14/review/implementation-blocker-66.md`

最新修复候选见 blocker 66：exact run `34295522872` 的 x86 historical replay 失败由
v0.13 `4add225778b867e33227236d178de33139a97d36` 的受限 v4 integer-map width 修复吸收。
AArch64 schema-9 tune-use compile 失败则通过保留 source-backed space、independently
checked plan replay 与 Native target/header，去除重复 discovery/materialization 和无用
ordinary artifact emission。完整 frontier 与全部 identity/legality/artifact 检查仍执行；
本地用例仅作诊断，新 exact-SHA 十作业 CI 尚须完成验收。

实施分支为 `design/v0.14-offline-autotuning`，独立 worktree 为
`.worktrees/v0.14-offline-autotuning-design`，通过审查并固化证据的起点为
`1f27df4b7992f1209f6762aeb11632509d888ae0`。v0.14 最初基于 v0.13 候选
`94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`；最终接纳的 v0.13 修订为
`4add225778b867e33227236d178de33139a97d36`。两者之间的累计提交已按
`implementation-design-correction-10.md` 逐文件复核并以等价或更严格的 v0.14 实现吸收；
历史 replay 也固定到该最终 SHA，移动分支、tag 或旧 CI 不得替代此身份。

Exact V0.14 run `34123758500` 的 AArch64 performance job `101747821716` 在重建
exact V0.13 时以 multiversion source-to-object geometric ratio
`2.5245647 > 2.5` 失败。复诊与闭环见
`specs/0.14/review/implementation-blocker-45.md`：V0.13 基线前移到
`7b883bf36a2edfb6720caa69aa7f10c94ebb9e43`，normalized KIR body 改用结构相等性，
一次 independent-check authority 保留到 emission，coverage-first retention 与
predicted-cost-first runtime dispatch 分离。性能/稳定性门槛、timed work、样本、corpus、平台与
required job matrix 均未改变。

Exact V0.14 run `34133617471` 随后重放了 V0.13 的 checked x86 map 有害展开、Unix private
runtime ident 体积、Windows stripped-PE 观察点和 ARM64 `_Interlocked*` 链接问题。复诊与继承
闭环见 `specs/0.14/review/implementation-blocker-46.md`：V0.14 精确吸收 V0.13
`6fd8234859dfe667419b7be9e601ad79426fd2dd` 并将 replay manifest 重钉到该 SHA，manifest
SHA-256 为 `138f4fe15331698f8b14c1b5fba56057d4935dfbbe1948fd5f22f935ee932932`。
同一 run 的 x86 worker 只有 v3、缺少 schema 9 规范要求的 v4；该环境失败保持 hard fail，不能以
降低 required tier、skip 或模拟计时规避。语言/公开 ABI、安全语义、目标 ISA、schema 9、性能/
稳定性/产物门槛、timed work、样本、corpus、平台与 required job matrix 均未改变。

Exact V0.13 run `34155662442` 的 ARM64 Native jobs 证明新增 helper regression 误走 ordinary
KIR O3，x86-64 performance job 同时证明 checked constant-bound map 被 streaming-map
`unroll.disable` 覆盖。Exact V0.14 run `34155664658` 在两套 ARM64 Native jobs 复现同一
test-path 缺陷。复诊与继承闭环见
`specs/0.14/review/implementation-blocker-47.md`：V0.14 精确吸收 V0.13
`e869763366283e46cd76ffbf3bb85c6c3959c25c`，Native regression 改走真实 multiversion KIR
管线；checked constant-call map 恢复既有有界二路 schedule，unknown-length checked streaming
map 继续禁止有害展开。replay manifest 重钉到该 SHA，SHA-256 为
`4e9b37ae4687fa5f11c3da029e57fd3e1e6bd9512a2b66bd8599de9fd2337c3d`。语言/公开 ABI、安全语义、目标 ISA、schema 9、inline/growth budget、性能/
稳定性/产物门槛、timed work、样本、corpus、平台与 required job matrix 均未改变。

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

Exact v0.14 run `34087252444` 的 AArch64 performance job `101633869632` 在 replay
准备成功后，因当前 v0.14 `checked/modular_reduction` 仅达到较快 SIMD oracle 的
`85.85%` 而失败。下载的 KIR 与机器码证明 v0.14 遗漏了 accepted v0.13 corpus 源码已冻结的
`unsafe` 合约 `requires n <= a.len; effects read(a);`，从而在每次迭代保留额外边界退出；同一
worker 独立重建的 v0.13 合约版本为 8,253,184 ns，v0.14 遗漏版本为 10,527,912 ns。V0.14
已恢复 exact v0.13 源码字节及 manifest SHA-256
`00e9cf6faf936e510929c1d4352bbaa41d3a24cd837194dbd651e5059f141025`，并以 source-byte 与
Native trusted-contract 回归锁定。复诊见 `specs/0.14/review/implementation-blocker-35.md`；
语言/公开 ABI、安全模式、oracle、工作量、样本、统计方法、平台、required job 与全部门槛不变。

Exact v0.14 run `34090234424` 的 AArch64 performance job `101642274522`
通过累计 schema 7/8 后，在 schema-9 第一次 cold tune 前拒绝 collector 以默认 `0755` 创建的
`cache/branch-layout/cold-one/ckc`。复诊与闭环见
`specs/0.14/review/implementation-blocker-36.md`：collector-owned cache namespace 现在显式以
POSIX `0700` 创建并在 snapshot 前复核模式；编译器既有 no-follow、owner-only 与 fail-closed
安全契约未放宽。语言/公开 ABI、tuning choice、cache key、工作量、样本、统计方法、平台、
required job 与全部门槛不变。

同一 exact v0.14 run `34090234424` 的 x86-64 performance job `101642274739`
重建 accepted v0.13 replay 时，Dijkstra KIR optimizer 以 `2,531,741 / 832,254 =
3.041x` 超过未改变的单项 `3.0x` 门槛；前一 v0.13 exact run 的 `2.951x` 仅有不足够的
波动余量。V0.13 已在 `60e26ac01444903180b90ee3bf7da08c905c0915` 将 phi pruning 中
仅用于键查询的短生命周期有序映射改为 `HashMap`，保留确定性 live-value set 与全部 KIR
语义。V0.14 已精确继承该修复，并把 replay accepted commit 与 manifest SHA-256 重钉为
`a8feea2ad72cfdae9135ff5ba43add8071fbdb7344b3021d2c03aa243aa4eddf`。复诊见
`specs/0.14/review/implementation-blocker-37.md`；工作量、样本、corpus、平台、required job、
性能与稳定性门槛均保持不变。

Exact v0.14 run `34095419897` 的 x86-64 performance job `101658156635`
重建 accepted v0.13 `60e26ac01444903180b90ee3bf7da08c905c0915` 后，unchecked
`integer_cast` 仅达到较快 Rust SIMD oracle 约 `87.19%`，未通过不变的 `90%` 门槛。
稳定样本、KIR 与反汇编证明 `VF2/UF4` 的多指令 `u32 -> f64` legalization 把热循环扩大到
约 98 bytes，而 oracle 的同语义双链热循环约 52 bytes。V0.13 已在
`002100719bdefdabb0fece50a363e1b797c464d2` 加入 x86 widening-cast 两链 frontend budget；
`UF4` 仍被发现、物化和独立检查，普通与三流 integer map 仍使用四链。V0.14 精确继承该修复，
同步更新 ordinary/multiversion object cache identity，并把 replay manifest SHA-256 重钉为
`2304bb4a6dab1ef060b37200063f1d33f34429c6cf03ad08e774110f91604eca`。复诊见
`specs/0.14/review/implementation-blocker-38.md`；语言/公开 ABI、安全语义、目标 ISA、候选
frontier、工作量、样本、corpus、平台、required job、性能与稳定性门槛均保持不变。

Exact v0.14 run `34100659848` 的 x86-64 performance job `101674461128`
重建 accepted v0.13 `002100719bdefdabb0fece50a363e1b797c464d2` 后，
`branch-layout` generation 以 `782,368 / 152,662 = 5.1248x` 未通过不变的
`5.0x` overhead 门槛。稳定样本与 artifact 反汇编证明，LLVM 内联两个 internal helper
算术后，每个 loop element 仍保留两次 function-entry atomic publication。V0.13 已在
`280388a396e85f8876300427cd56b627b08b3e45` 将 internal、non-exported、non-entry
function-entry observation 精确迁移到 static call-site caller-local 饱和计数器，并在
return 时 bulk-add 发布；export 与 module entry 保持直接发布。V0.14 已精确继承该修复，
并把 replay manifest SHA-256 重钉为
`81a052192afc6ae7d124f6ae36e56636e0c0d26b72f29c510a98a41dab559d17`。复诊见
`specs/0.14/review/implementation-blocker-39.md`；语言/公开 ABI、安全语义、profile schema、
目标 ISA、工作量、样本、corpus、平台、required job、性能与稳定性门槛均保持不变。

Exact v0.14 run `34106159689` 的 x86-64 performance job `101691955900`
在 schema-7/schema-8 成功后，schema-9 的 `contract-fixed-length` 两个独立 cold session
分别选择 tuned plan `493f1128...` 与 `validation-disagreement` baseline。完整 raw streams
证明所有相关流稳定且候选通过既有收益门槛，但当前 exact-Q32 排名把远低于一个百分点的纳秒
波动置于 artifact/choice/digest 确定性键之前，同时 search entrant 集也随之改变。复诊见
`specs/0.14/review/implementation-blocker-40.md`：search/validation 现以 checked u128 派生
`ceil(score_q32 * 100 / 2^32)` 的冻结百分点上取整桶，再按 artifact bytes、choice count、
plan digest 全序。精确 Q32 记录、0.97/1.02 门槛、16/20 paired wins、工作量、样本、corpus、
平台和 required job 均保持不变。

同一 exact run 的 AArch64 performance job `101691955647` 在成功写出完整 schema-9
evidence 后，detached v0.13 checker 因 outer evidence root 保持相对路径而从 child checkout
错误解析历史 report。artifact 已证明该 report 与完整 tree 实际存在。复诊见
`specs/0.14/review/implementation-blocker-41.md`：跨入 detached checkout 前，checker 现在把
evidence root 绝对化，并以 outer owner 下的 absolute report argument 调用历史 checker；identity、
byte、tree、symlink、commit 与 checker 等字节验证不变。性能/稳定性门槛、timed work、样本、corpus、
平台和 required job 均保持不变。

Exact v0.13 run `34106156091` 的 x86-64 performance job `101691953958` 以
`14142 / 13272 = 1.0656 > 1.05` 未通过 `trip-unroll-simd` 的 dispatch/direct
门槛。稳定 samples、byte-identical selected-direct artifact 与反汇编证明 one-shot publish
完成后，公开 dispatcher 仍逐调用执行 acquire load、null test、conditional branch 与 indirect
jump。V0.14 已逐字继承 exact v0.13
`7a292bc74a87591437fc87ceb06df1fbb1bb28fd` 的 resolver-sentinel closure：slot 初值是
exact-ABI cold resolver entry，首次调用沿既有 acquire-release publication 获得 winner 并
must-tail 调用，稳态只保留 acquire load + indirect must-tail call；codegen/cache identity 同步
更新。replay manifest 重钉到该 SHA，SHA-256 为
`0cd154de78d8b305e73f9707ee8236fdb6064977b4f0be136a814d1a4fdda6e2`。复诊见
`specs/0.14/review/implementation-blocker-42.md`；V0.13 仍独立验收，语言/公开 ABI、profile
格式、target ISA、性能/稳定性门槛、timed work、样本、corpus、平台与 required job 均不变。

Replacement exact v0.13 run `34116187720` 在 native integration、x86-64 Linux 与
AArch64 Darwin 一致证明 resolver entry 的 inherited fact lineage 少登记一份：LLVM 正确携带
四份 readonly/writeonly，而 CK ledger 只预期三份。V0.14 已精确继承 exact v0.13
`dd6239e677720845dee874ac3095710941b58d5b`，为 cold resolver entry 与 steady dispatcher
分别登记 closed inherited property set，同时继续禁止复制 body-owned evidence；v0.14 独有的
profile-runtime contract 在机械冲突处理时完整保留。replay manifest 重钉到该 SHA，SHA-256 为
`cc16808de13642e65c843668e60ac93ca2db0d0c6b4d1247346544fe5bcfcea3`。复诊见
`specs/0.14/review/implementation-blocker-43.md`；fact-audit equality、语言/ABI、性能/稳定性
门槛、timed work、样本、corpus、平台与 required job 均不变。

Exact v0.13 run `34117792378` 的 x86-64 performance job `101728726666` 随后证明，
coverage-first retained-set 在只按每个 target profile 的完整 clone 计费时，仅能保留 v3，
使具备 AVX-512 的 v4 runner 仍执行 AVX2 member 并未通过不变的 compute-bound 90% PGO
oracle 门槛。V0.14 已精确继承 exact v0.13
`0b2eaa52682d06300a009b2378a9ce00697f93f5`：只有 target profile 与 tier-derived hidden name
不同、规范化后完整 KIR bytes 一致的 member 才共享一次 logical body charge；任何真实结构差异
仍完整计费，各 target module/object/audit/artifact gate 仍独立。replay manifest 重钉到该 SHA，
SHA-256 为 `b2fb99873ac107481bac79529b4ac1bdc5e63abbbd2a8e925d6e9c935c612ffd`。
复诊见 `specs/0.14/review/implementation-blocker-44.md`；`2x` logical KIR growth、语言/ABI、
安全语义、target ISA、性能/稳定性门槛、timed work、样本、corpus、平台与 required job 均不变。

Exact V0.14 run `34133617471` 在重建 V0.13 `7b883bf36a2edfb6720caa69aa7f10c94ebb9e43`
时复现了 checked x86 streaming map 有害二路展开、Unix private runtime ident 聚合体积、Windows
stripped-PE 私有符号误判和 ARM64 `_Interlocked*` 未展开链接失败。V0.14 已精确继承 V0.13
`6fd8234859dfe667419b7be9e601ad79426fd2dd` 的闭环并重钉 replay manifest；复诊见
`specs/0.14/review/implementation-blocker-46.md`。同 run 的 x86 runner 只有 v3 而缺少规范要求的
v4，保持 required-capability hard fail，等待 replacement exact-SHA run 分配真实 v4 host。
语言/公开 ABI、安全语义、target ISA、schema 9、性能/稳定性/产物门槛、timed work、样本、
corpus、平台与 required job 均不变。

Exact V0.13 run `34155662442` 的 ARM64 Native jobs 证明新增 helper regression 误走 ordinary
KIR O3，x86-64 performance job 则以 checked `specialized_length`
`4,860,459 / 4,052,972 ns` 未通过不变的 90% throughput 门槛。Exact V0.14 run
`34155664658` 在两套 ARM64 Native jobs 复现同一 test-path 缺陷。V0.14 已精确继承 V0.13
`e869763366283e46cd76ffbf3bb85c6c3959c25c`：regression 使用真实 multiversion KIR 管线并先
验证 helper 保留状态；checked constant-call map 使用有界二路 schedule，unknown-length checked
streaming map 继续禁止有害展开；replay manifest SHA-256 更新为
`4e9b37ae4687fa5f11c3da029e57fd3e1e6bd9512a2b66bd8599de9fd2337c3d`。复诊见
`specs/0.14/review/implementation-blocker-47.md`；语言/公开 ABI、安全语义、target ISA、
schema 9、inline/growth budget、性能/稳定性/产物门槛、timed work、样本、corpus、平台与
required job 均不变。

Exact V0.14 run `34165564989` 随后重建 V0.13
`e869763366283e46cd76ffbf3bb85c6c3959c25c`，x86-64 checked domain suites 以
`1.0413218x` 与 `1.0415426x` 未通过不变的 `1.05x` 几何门槛。完整样本稳定，机器码证明固定
长度契约在 pre-O3 alloca IR 中被误判为 unknown-length streaming map 并附加
`unroll.disable`。V0.14 已按 `specs/0.14/review/implementation-blocker-48.md` 精确继承 V0.13
`aa757ca0f78664cbfa4f824d655d820c87368dd3`：analysis clone 先经 mem2reg，再从 constant
direct call 或 `llvm.assume(n == constant)` 恢复固定边界；仅固定边界使用有界二路 schedule，
未知长度 checked streaming map 保持禁止有害展开。replay manifest SHA-256 重钉为
`d58fd2cbaaf35fb611bd666b0e027f4467291d06a27bb073ddfb54d431062898`。语言/公开 ABI、
安全语义、target ISA、schema 9、性能/稳定性/产物门槛、timed work、样本、corpus、平台与
required job 均不变。

Replacement V0.14 exact run `34169415571` 的 x86-64 performance job
`101886905457` 随后在历史 V0.13 schema-7 重放中，以 checked `strict_f64`
`3,601,329 / 3,164,344 ns` 未通过不变的 90% 单项吞吐门槛。前一 run 对应结果为
`3,194,955 / 3,191,369 ns`，而 candidate、Rust oracle 动态库与 candidate 反汇编均逐字节
相同。复诊与闭环见 `specs/0.14/review/implementation-blocker-49.md`：V0.14 精确继承 V0.13
`1aad5bdd964f3afa4b367434c1c3810fb63f8e8f` 的共享 `KernelWorkspace`，让三条已加载 entry
复用同一输入输出地址，消除分配位置造成的持久通道偏差；sampling identity 更新为
`interleaved-upper-median-three-channel-v3`，oracle manifest SHA-256 为
`e4e8e4e70893a81cb96f8d7e0e5dbc1e5f971236ee88b3d0b2e2c55fdda854b3`，V0.13 replay
manifest SHA-256 重钉为
`578868dabbba1a10267c1500269fe75b1e953a3ef913ba71612257c196478fdb`。语言/公开 ABI、
strict-FP、安全语义、target ISA、schema 9、优化策略、性能/稳定性/产物门槛、timed work、
样本、corpus、平台与 required job 均未改变。

Exact V0.14 run `34172973863` 随后在两个 performance jobs 的历史 schema-8 重放中
暴露两个独立问题：x86-64 `memory-bound` multiversion/ordinary 为
`1.04241 > 1.03`，但 multiversion/selected-direct 为 `0.99547`；AArch64
multiversion source-to-object 几何均值为 `2.52084 > 2.5`。复诊与继承闭环见
`specs/0.14/review/implementation-blocker-50.md`：V0.14 精确吸收 V0.13
`21448738b90ccfd1ea9ab79e9355450ef325769c`，schema-8 八通道共享唯一工作区并更新 sampling
identity 为 `rotating-eight-channel-shared-workspace-v2`；checked multiversion emission
复用主管线已验证 baseline，raw public emitter 与 enhanced variants 仍 fail closed。V0.13
replay manifest SHA-256 重钉为
`8852346e6263c3dcf86a30ef2b0084e029865f9deb0545231f73169f0982b148`。语言/公开 ABI、
strict-FP、安全语义、target ISA、schema 9、优化策略、性能/稳定性/产物门槛、timed work、
样本、corpus、平台与 required job 均未改变。

Exact V0.13 run `34178811720` 随后在两个 performance jobs 的 PGO oracle 审计中一致失败：
schema-8 已要求 `KernelWorkspace`，但审计仍向 `Kernel` 传入原始 record dict。V0.14 exact run
`34178814878` 因而同时携带过时审计和已被拒绝的 replay pin。复诊与继承闭环见
`specs/0.14/review/implementation-blocker-51.md`：V0.14 精确吸收 V0.13
`77e5e0a95b83d0faa8f63ddc8f2451a9b1322a40`，三种 PGO oracle 共享每个 record 的唯一 workspace；
accepted-base 与 replay 重钉到同一 SHA，manifest SHA-256 为
`9cb05a28b504e504ccd6dca50617241e11a29a8e8d9034b35f4d2b72d181dfe8`。语言/公开 ABI、
strict-FP、安全语义、target ISA、schema 8/9、性能/稳定性门槛、timed work、样本、corpus、平台
与 required job 均未改变。

Exact V0.14 run `34182332164` 的两个 performance jobs 已通过不变的累计 schema-7/8
门禁。AArch64 job `101923906526` 随后证明，schema 9 在 detached V0.13 checkout 内运行
保留 checker 时错误继承外层 V0.14 `GITHUB_SHA`，使正确的历史证据被身份校验拒绝；x86-64
job `101923906271` 则被分配到仅支持 v3 的 AMD EPYC 7763，按冻结的 v4 要求正确 hard fail。
复诊与代码闭环见 `specs/0.14/review/implementation-blocker-52.md`：历史 checker 子进程现将
`GITHUB_SHA` 精确绑定到 `v013ReplayBundle.commit`，并由 focused regression 固定此边界；
replacement run 仍必须取得真实 v4 worker。历史证据、语言/公开 ABI、strict-FP、安全语义、
target ISA、schema 8/9、性能/稳定性/产物门槛、timed work、样本、corpus、平台与 required
job 均未改变。

Replacement V0.14 exact run `34187453075` 的 AArch64 performance job
`101938700087` 已越过 blocker 52 的历史 checkout SHA 校验，随后证明 retained V0.13 checker
仍继承外层 V0.14 的 `CKC_V012_RUNTIME_BUNDLE`、`CKC_V011_RUNTIME_BUNDLE` 与
`CKC_V010_RUNTIME_BUNDLE`，把 recipe `104e7f…` 错当成 V0.13 历史 closure 的
`ac4fda…`。复诊与闭环见 `specs/0.14/review/implementation-blocker-53.md`：三条环境路径现
精确绑定到 retained schema-8 report 同级的 `replay-v012`、`replay-v011`、`replay-v010`
目录；x86-64 job `101938700084` 再次取得仅 v3 的 AMD EPYC 7763，仍按冻结 v4 要求 hard
fail。历史证据、语言/公开 ABI、strict-FP、安全语义、target ISA、schema 8/9、性能/稳定性/
产物门槛、timed work、样本、corpus、平台与 required job 均未改变。

Exact V0.13 run `34182330156` 随后暴露 Windows canonical verbatim root 的 component
遍历错误，V0.14 run `34192455322` 因而固定了已被拒绝的历史依赖；该 V0.14 AArch64 run
还以 `2.5002854475` 未通过不变的 schema-8 multiversion compile `2.5` 门槛。依赖复诊见
`specs/0.14/review/implementation-blocker-54.md`：V0.14 已精确吸收 V0.13
`528f0734a0c4525a2c84158c4d73067e468f292c` 的 Windows root 修复，accepted-base 与独立
replay 重钉到该 SHA，manifest SHA-256 为
`2b2d2e66333b3eed4b8bd260325f3440e0040e6cdf43c1f4d81546eb5756eba4`。被替代的性能结果未
通过修改门槛、样本、timed work 或选择性 replay 掩盖；replacement exact-SHA workflow 必须
完整重建并重测。语言/公开 ABI、strict-FP、安全语义、target ISA、schema 8/9、性能/稳定性/
产物门槛、timed work、样本、corpus、平台与 required job 均未改变。

Exact V0.14 run `34198065606` 的 AArch64 performance job `101970492265` 在重建
accepted V0.13 schema-8 replay 时，以 multiversion source-to-object 几何均值
`2.500001057595058 > 2.5` 失败。完整十五样本覆盖证明不存在缺失或选择性重放；同一
accepted SHA 的独立 V0.13 AArch64 job 为 `2.35826535437881`，差异来自 retained V0.14
环境对 ordinary 路径的相对加速。复诊与继承闭环见
`specs/0.14/review/implementation-blocker-55.md`：V0.14 精确吸收 V0.13
`6258089cf44ebc317247e3fc38e2c765e424132e`，checked multiversion variant 在 opaque bundle
checker 已完成独立结构验证后复用 O0-shaped handoff，LLVM lowering 前的最终 evidence
validation 保持不变；accepted-base 与 replay 重钉到该 SHA，manifest SHA-256 为
`6a3f2768b56c737d6060d0f7ed03a103ed7570c4064c6cfe9532b2a91d23d230`。语言/公开 ABI、
strict-FP、安全语义、target ISA、schema 8/9、性能/稳定性/产物门槛、timed work、样本、
corpus、平台与 required job 均未改变。

Exact V0.14 run `34212249513` 随后在完整 `x86-64-v4` worker 重建 accepted V0.13
`6258089cf44ebc317247e3fc38e2c765e424132e`，compute-bound combined CK 为 34,832 ns，
Rust PGO oracle 为 30,025 ns，仅达到约 86.2% throughput，未通过不变的 90% 门槛；保留
v4 object 使用四条 256-bit YMM 链，而 oracle 使用四条 512-bit ZMM 链。复诊与继承闭环见
`specs/0.14/review/implementation-blocker-56.md`：V0.14 精确吸收 V0.13 blocker 45，
只对完整 v4、显式 `+avx512f`、至少八个 strict scalar `f64` 运算且无 fast-math/既有
schedule 的 compute-dense memory-map loop 授权八路 vector width；accepted-base 与 replay
重钉到 `e4f3fc6388a3ebd15cb1beb4ecd4dd9ca55b0dbe`，manifest SHA-256 为
`1de279a1972fcad7a9447d8e7281dd86aa9faf49fd514c181ffa82936294fa20`。语言/公开 ABI、
strict-FP/安全语义、target eligibility、schema、性能/稳定性/size 门槛、timed work、样本、
corpus、平台与 required job matrix 均未改变；V0.13 仍须独立通过自己的十作业验收。

Exact V0.14 run `34217917663` 随后在两个 performance job 暴露独立结果。AArch64 job
`102034356856` 完成全部 schema-9 采集后，checker 以 `branch-layout disagrees with decoded
decision` 失败；逐字段复诊证明唯一差异是 collector 把 inspector 的 textual `u64` byte count
规范为 JSON integer，而 checker 保留为 string。x86-64 job `102034357269` 已通过 schema 8，
随后因 AMD EPYC 7763 仅有 v3、没有 AVX-512 而按冻结规则拒绝 schema 9。复诊与闭环见
`specs/0.14/review/implementation-blocker-57.md`：checker 现在将 inspected byte count 经
既有 u64 range checker 规范为 JSON integer 后再比较；x86 v4 要求保持 hard fail，并由
replacement exact-SHA run 获取真实 v4 worker。语言/公开 ABI、strict-FP/安全语义、tuning
decision、证据字段、schema 版本、target eligibility、性能/稳定性/size 门槛、timed work、
样本、corpus、平台与 required job matrix 均未改变。

Replacement exact V0.14 run `34225333284` 的 AArch64 performance job
`102058308732` 完成 schema 8 与全部 schema-9 采集后，独立 checker 拒绝把 evidence-root
profile 排在 repository-root source 之前的混合根命令输入；完整报告复诊证明 producer 错把
root 名称按文本排序，而冻结 schema 要求 repository 后 evidence 的显式语义顺序。x86-64
job `102058308829` 同样已通过 schema 8，随后因 AMD EPYC 7763 仅有 v3、没有 AVX-512
而按冻结规则拒绝 schema 9。复诊与闭环见
`specs/0.14/review/implementation-blocker-58.md`：producer 与独立 checker 的各自实现现在
使用相同的显式 root rank 后接 UTF-8 path bytes，并由 RED/GREEN 混合根回归锁定；x86 v4
要求保持 hard fail，由 replacement exact-SHA run 获取真实 v4 worker。语言/公开 ABI、
strict-FP/安全语义、tuning decision、证据字段、schema 版本、target eligibility、性能/稳定性/
size 门槛、timed work、样本、corpus、平台与 required job matrix 均未改变。

## 当前 Schema-9 验收修订（规范优先于以上历史执行记录）

Schema-9 外层格式不变，当前 collector/checker 使用 `recipe.schema = 2`。Revision 1
报告仍以原阈值、`rotating-three-channel-v1` 验证通道和原判定语义可识别、可读取；
不得静默重解释。Revision 2 保持全部 timed work、warmup、20 samples、每样本 7 calls、
corpus、两平台与十个 required jobs，并将 validation 扩为
`rotating-four-channel-v2`。

版本回归只比较 v0.14 ordinary 与 exact v0.13 ordinary；Auto-Tuning 只比较同 SHA、
安全模式、目标和输入下的 v0.14 tuned 与 v0.14 ordinary。可信逐项退化不得超过 3%，
ordinary 的可信 geometric aggregate 不得退化，tuned geometric aggregate 必须硬性至少持平。
selected tuned 必须在 validation 同时达到 3%
upper-median 和 16/20 paired-row 收益，否则发布 byte-identical ordinary fallback；
至少两个 release-held-out workload 必须重复该收益。

完整 v0.13 PGO channel/sample/profile/build/artifact 继续采集，但对无 PGO Auto-Tuning
仅作诊断。未来 PGO + Auto-Tuning 组合模式必须增加“不弱于对应 PGO”的硬门槛。
缺少 x86-64-v4/AVX-512 或 AArch64 SVE2 必须以带缺失 feature/CPU 的 runner
capability/infrastructure failure 失败关闭，不得 skip，也不得误报 compiler regression。

Exact V0.13 run `34241617859` 随后在 Windows ARM64 job `102113212994` 证明，
profile runtime 为 lock-free `std::atomic_ref` 使用 C++20 frontend 后，遗留的 C 风格
`(void *)0` 无法隐式转换为 `CreateFileW` 与 `WriteFile` 的具体 SDK 指针类型。复诊与继承
闭环见 `specs/0.14/review/implementation-blocker-59.md`：V0.14 对三个安全属性参数和一个
overlapped 参数应用同一精确类型修复并更新 provenance；accepted v0.13 base、replay manifest、
preparer 与当前验收文档重钉到独立修复提交
`8286b32174e33a4b874d71e8c17a5a605a382f0d`，manifest SHA-256 为
`26f30615b9e87f7c3d88d5dd14e00e635e891637792cc445a9ecd812d3a2fbae`。历史记录保留
旧身份作为证据；语言/公开 ABI、profile/schema 8/9、安全与 strict-FP 语义、优化与 tuning
策略、target eligibility、性能/稳定性/size 门槛、timed work、样本、corpus、平台与
required job matrix 均未改变。

Superseded exact V0.14 run `34252240969` 随后在 AArch64 performance、native
integration、Linux ARM64、Darwin ARM64 与 Linux x64 五个 required job 中一致于 prefix
bootstrap 失败。完整日志均指向 `ck_profile_atomic_u32_fetch_add_relaxed`：该操作只供 Windows
ARM64 run-id serial 使用，却在 AArch64 Linux 与 generic C11 分支生成未使用的 non-inline
internal function，因而触发冻结的 `-Werror=unused-function`。复诊与闭环见
`specs/0.14/review/implementation-blocker-60.md`：只把两个 Unix 分支的定义改为
`static inline`，函数体、memory order、assembly、Windows 定义与全部门槛保持不变，并更新
profile-runtime provenance digest。先红后绿的 branch-specific contract 与本机冻结 flags 的
Darwin C11 直接编译均通过；语言/公开 ABI、profile/schema 8/9、优化与 tuning 策略、target
eligibility、性能/稳定性/size 门槛、timed work、样本、corpus、平台与 required job matrix
均未改变。

Exact V0.14 run `34258812502` 的 AArch64 performance job `102171708973` 完成
schema 8 和完整 schema-9 采集后，在不变的 domain throughput 门槛中暴露
`contract-fixed-length` 的真实 codegen 回归：CK ordinary 与 tuned fallback 均为
`89,046,408 ns`，generic C 为 `84,473,908 ns`；反汇编显示 CK 将已证明 `n == 16`
的循环完整展开为 16 个标量 add，而 generic C 保留循环并形成 SVE vector loop。复诊与
闭环见 `specs/0.14/review/implementation-blocker-61.md`：AArch64 SVE Native handoff
现在只对具有常量等式 bound、trip count 至少 16 且能整除四 lane/四路 interleave chunk
的 32-bit integer scalar memory map 设置 fixed-width vectorization、四路 interleave 与
`unroll.disable`；动态 SVE loop 保持既有策略。ordinary 与 tuned cache identity 均绑定该
object-affecting 修复。语言/公开 ABI、strict-FP/安全语义、target eligibility、schema 8/9、
性能/稳定性/size 门槛、timed work、warmup、样本、corpus、平台与 required job matrix
均未改变。

同一 exact V0.14 run 的 Windows x64 native job `102171709543` 在六个 publication/
recovery 测试首次建立 persistent lock 时均以 `protect Windows publication file: Access is
denied (os error 5)` 失败。复诊与闭环见
`specs/0.14/review/implementation-blocker-62.md`：`SetSecurityInfo` 写 DACL 要求 handle
具有 `WRITE_DAC`，而旧 private initializer 只有 generic read/write；Windows 创建访问掩码
现在显式包含 `WRITE_DAC`，并保留 ACL 失败即删除未保护 initializer 的 fail-closed 行为。
同一 run 的 x86 performance job `102171709329` 仍因 AMD EPYC 7763 仅有 v3、缺少
AVX-512 而正确报告 runner capability/infrastructure failure，replacement run 必须取得真实
v4 worker。语言/公开 ABI、publication/schema 8/9、安全语义、优化与 tuning 策略、target
eligibility、性能/稳定性/size 门槛、timed work、warmup、样本、corpus、平台与 required job
matrix 均未改变。

最新 exact V0.14 run `34281039339` 的 AArch64 replay 在不变的 `2.5` compile-time
门槛失败；Windows PGO 的 verbatim-prefix 与 ARM64 outlined atomics 也需继承修复。
当前 accepted V0.13 与 replay 已统一前移到
`f5dd9989245fd6d9e70babc95dcdf7af17ecb42f`，manifest SHA-256 为
`229fecf4dd95610ae67309b683142b53f014abf4db655611b188e8330aca11f3`。
精确差异及完整 target/profile/digest 等价验证见 `implementation-blocker-63.md`；既有
V0.14 tuning、publication、Unix runtime 修复保持不变，历史记录不改写。

同一 run 的 Windows x64 publication 在 owner-only ACL 设置之后仍触发无上下文的
AccessDenied。`implementation-blocker-64.md` 修复 directory flush handle 缺少
`GENERIC_WRITE` 的 API 违约，并区分 open/flush error。两个 Windows job 在 LLVM
bootstrap 前增加同一真实 publication selector，原 post-bootstrap native tests 全部保留。
不忽略 directory barrier failure，不增加 CRT，不降低任何平台、硬件、性能或安全门槛。

Exact V0.14 run `34291739279` 的两个 Windows preflight 均已通过 publication、完整 crash
matrix 和真实 killed-session recovery；仅 lock identity observer 仍在拿锁期间经第二 handle
读取内容，触发符合 Win32 语义的 error 33。`implementation-blocker-65.md` 同时修复生产
acquire 路径的 read-before-lock：身份校验改为持锁后在 owning handle 上执行，竞争 waiter
先等待再校验；测试增加 Windows 锁内 second-handle 拒读断言，unlock 后保留全部持久字节
断言。真实并发回归先在本机复现 early identity rejection，再验证 waiter 在 release 后成功。
Windows preflight 显式执行该 unit regression 与原 publication selector，矩阵与门槛不变。
