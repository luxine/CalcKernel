# CK 0.13 PGO 与 CPU 多版本实施总控

## 固定输入与交付边界

本计划实现已经通过三轮对抗性审查的双语规范：

- `specs/0.13/profile-guided-multiversioning.md`
- `specs/0.13/zh-CN/profile-guided-multiversioning.md`
- `specs/0.13/review/design-adversarial-review-01.md`
- `specs/0.13/review/design-adversarial-review-02.md`
- `specs/0.13/review/design-adversarial-review-03.md`

实施分支是 `design/v0.13-pgo-multiversion`，独立 worktree 是
`.worktrees/v0.13-pgo-multiversion-design`，设计审查通过的起点为
`65f2b0fe25c130106e65d7cdd4c8156b8fac3b33`。性能 replay 固定使用 CK 0.12 候选
`ea822e343967baa2db113d3dd8f429d8dfdfa779`，不得用移动分支、tag 或本机现有二进制代替。

目标是在该分支形成完整、可审查的 0.13.0 候选并提交。不得自动合并 `main`，不得创建或
移动 tag，不得创建 GitHub Release。exact-SHA 远程验收可以推送该分支并显式触发 CI；长时间
作业只做间隔查询，不在前台持续等待。

## 不可变执行规则

1. 计划提交后完全行内执行，不再使用子代理。
2. 严格 TDD：每项行为先写最小测试并实际观察与缺失能力一致的 RED，再实现 GREEN，最后
   重构。schema migration 的编译错误可以作为 RED，但必须确认错误来自新增契约。
3. 阶段 01–11 必须顺序执行；本阶段 task 全部完成且 acceptance 全部通过后才能进入下一阶段。
4. PGO 默认关闭；不改变 CK source syntax、semantic MIR、strict `f64`、checked first-error、
   effect/print 顺序、slice ABI、Native ABI 1 或 Runtime ABI 2。
5. profile 只能提供收益信息，不能提供安全证明。每个 PGO/variant proposal 必须由不调用
   proposer 的 checker 从 pre-state、closed record、静态 proof、profile record 与 target cost
   重新计算；未知、饱和、溢出、歧义或 mapping 丢失都回退 baseline。
6. 任何 variant 均从同一 verified logical KIR pre-state 开始；跨 variant LTO 禁止。trial 拒绝、
   reuse、非获胜 candidate 与预算耗尽不退款，且不能污染已验证 state。
7. O2 的 profile 权限只限于已冻结 ordinary machine pipeline 后的 CK late layout；不得把
   profile metadata/attribute 交给 LLVM，不得借修复名义改变非 terminator 指令或结构。
8. 不为通过功能、性能、size、compile-time 或 CI 而降低门槛、删语料、忽略测试、扩大 repair
   allowlist 或改变统计。只有真实规范反例可先复诊，再同步修改双语规范、总控、相关 task/
   acceptance 与测试。
9. 生成物只进入已忽略的 `target/`、`build/` 或显式临时目录；不得提交 LLVM prefix、profile
   shard、benchmark 报告、CI artifact 或本地 agent 文件。
10. 每阶段开始前确认 branch/worktree；完成时在 `target/acceptance/v0.13/stage-NN/` 记录 SHA、
    RED 摘要、命令、测试计数、Rust/LLVM/Clang/host identity。旧日志不能代替当前 SHA。

## 冻结实现架构

```text
CheckedProgram -> semantic MIR -> scalar KIR 3 + canonical site table
  -> off | fixed generate instrumentation | validated profile sidecar
  -> ordinary verified O1/O2 prefix
  -> O2: ordinary machine snapshot -> CkLateProfileLayout -> emit
  -> O3: PGO analysis -> checked specialization/loop/SIMD transactions
  -> immutable verified baseline -> checked target variants
  -> separate LLVM modules -> feature audits -> baseline-safe dispatcher
  -> named-object assembler -> CKCOBJ03/cache manifest 4 -> final artifact
```

新公共编译器子系统位于 `src/profile/`，与现有 target profile
`src/ir/kir/profile.rs` 分开：前者表示 CK workload profile，后者仍表示 LLVM/target cost profile。
profile count sidecar 是 immutable non-proof analysis；安全事实仍只来自既有 fact/proof arena。

generation runtime 与 dispatch runtime 是 compiler-private support，身份独立进入 cache/manifest，
不提升 Native ABI 1 或 Runtime ABI 2。generation artifact 不进 object cache；use/multiversion bundle
只有 dispatcher manifest 和全部 named variant objects 一致时才命中。

## 阶段顺序

| 阶段 | 交付物 | 主要仓库落点 | 前置 |
| --- | --- | --- | --- |
| 01 | canonical identity、CKPROF01/CKPART01、merge/inspect 与 CLI 闭集 | `src/profile`, `src/cli` | 无 |
| 02 | KIR 3 site table、profile effect/instrumentation op、mapping verifier | `src/ir/kir`, `src/optimizer` | 01 |
| 03 | generation pipeline、private collector、目录锚定、自动/显式 flush | `native/profile_runtime`, `build.rs`, `src/cli` | 02 |
| 04 | profile application、confidence/work/cost、mapping transfer 与 explanation | `src/profile`, `src/optimizer` | 03 |
| 05 | O2 CK late machine layout、bridge ABI 4 与 target repair allowlist | `native/bridge`, `src/backend/llvm` | 04 |
| 06 | O3 PGO specialization/inlining/loop/SIMD 的 checker/transaction 集成 | `src/optimizer` | 05 |
| 07 | multiversion target set、variant planner/checker、预算与 KIR bundle | `src/backend/llvm`, `src/optimizer`, `src/ir/kir` | 06 |
| 08 | baseline-safe detector/dispatcher/thunk、并发缓存与 ABI/feature audit | `native/dispatch_runtime`, Native backend | 07 |
| 09 | named-object artifact assembler、CKCOBJ03/key+manifest 4、CLI 原子输出 | artifact/cache/CLI | 08 |
| 10 | 0.13.0 identity、兼容矩阵、current docs、release/audit contract | Cargo/docs/contracts | 09 |
| 11 | schema 8 benchmark、0.12 replay、PGO/oracle/dispatch/size/time 与十作业 CI | benches/scripts/workflow | 10 |

阶段 01–11 各有同号 `*-task.md` 与 `*-acceptance.md`。`99-final-acceptance.md` 是唯一总验收
清单；阶段通过不能代签后续六主机、性能或 exact-SHA CI。

## 提交与远程策略

审查结论和全部计划先单独提交。实现阶段提交信息使用：

    compiler(stage-NN): <imperative outcome>

阶段内可保留小型 RED/GREEN 提交，但每阶段至少形成一个清晰检查点。动态证据只写 ignored
acceptance bundle 与 CI artifact，不能把 run id/final SHA 回写进被测提交造成自引用。任何影响
代码、规范、计划、fixture 或 checker 的新提交都会产生新 SHA，受影响门禁必须重跑。

远程 CI 仅在本地所有可运行验收通过且 feature branch 已推送后触发。使用
`gh workflow run ci.yml --ref design/v0.13-pgo-multiversion`，记录 run id 与 head SHA；每次查询
间隔至少 30–60 秒，在等待期间继续不依赖结果的本地工作。最终 worktree 必须干净并停留在
该分支，等待用户审查。

## 阻断处理

- 实现缺陷：保留最小 RED，修复实现，不改门槛。
- 测试缺陷：只有测试违背冻结规范或不能观察指定行为时修改，并记录具体反例。
- 环境缺陷：记录 host/Rust/LLVM/Clang identity 并修复环境；Native/CI gate 不得改成 skip。
- 规范反例：先写 `specs/0.13/review/implementation-blocker-NN.md` 复诊；成立后同步修订双语
  规范、总控、受影响阶段和测试，再继续执行。
- 远程缺陷：区分产品失败、runner/capability 失败与暂态基础设施失败；不得把 required job
  改为 optional 或用本地/旧 SHA 结果替代。

阶段 11 的首个 exact-SHA run 因 GitHub 仓库从 `Rust_CalcKernel` 更名为 `CalcKernel`，与原
TypeScript oracle 的旧仓库名发生碰撞，错误地在当前 Rust 仓库解析历史 commit。复诊记录见
`specs/0.13/review/implementation-blocker-01.md`。闭环只把该固定提交的最小 oracle 源码与
fixtures 固化到 `tests/oracles/typescript`，保留 lockfile、provenance 与 source manifest，并继续
执行原有 live differential gate；语言/ABI、性能阈值、corpus 与十作业要求均不变。

候选 `94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05` 的十作业 CI 随后暴露三个真实阻断：Linux
GCC `-Werror` 拒绝 profile-runtime 的 mixed-signedness 条件表达式；Darwin x86_64 profile
runtime 引用 `_fstat$INODE64`，但 freestanding `libSystem.tbd` 未导出该符号；同时 v0.13 仍
replay 已被 v0.12 后续 CI 证明有缺陷的旧候选。复诊与闭环见
`specs/0.13/review/implementation-blocker-02.md`。v0.13 已继承 v0.12 blocker 04 的修复并将
exact replay 重钉到 `ea822e343967baa2db113d3dd8f429d8dfdfa779`；语言/ABI、PGO 语义、
schema 8 门槛、corpus、统计与 required job topology 均不变。

Native 阶段使用仓库固定 LLVM/Clang 22.1.8 prefix 与 Rust 1.90.0。若本机缺失 prefix，先按
README/bootstrap manifest 恢复；阶段 03 之后的 Native acceptance 不能因此跳过。

Exact run `33795954634` 的双 performance 失败复诊见
`specs/0.13/review/implementation-blocker-09.md`：Linux schema 7 case 现在从 conditioning 到计时
固定在 inherited affinity 允许的一颗 CPU，schema 8 evidence 同时保留累计 schema 7 JSON 与其
引用的 `measurement-*` 目录。该闭环未改变 timed work、样本、统计量、门槛、corpus 或平台矩阵。

V0.14 run `33808562098` 在 exact V0.13 replay 中再次复现 same-core `slp_quad` 双频带，证明
affinity 仍不能排除共享宿主未调度 vCPU 对 wall-clock 的污染。复诊与闭环见
`specs/0.13/review/implementation-blocker-10.md`：继承的 Linux schema7 runtime sample 改用当前
线程 CPU time；timed work、样本、统计量、门槛、corpus 与平台矩阵保持不变。
