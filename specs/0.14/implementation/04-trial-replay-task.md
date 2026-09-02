# 阶段 04 任务：trial typestate、object/link 身份与 source-aware replay

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

把 compile selection 中的 legal plan 编译成不可发布 trial，冻结实际 primary bytes、object graph 与 link
recipe，并提供不信任 decision 的 source-aware checker 独立重建所有 trial 和最终 selected artifact。

## 仓库落点与接口

- 新建 `src/tune/{trial.rs,replay.rs,artifact.rs}`；`NonPublishableTuneTrial` 只向 runner staging 暴露
  private artifact，不能转换为 `OutputTransaction` 输入。
- 修改 `src/backend/llvm/{mod.rs,object.rs}`、`src/backend/artifact/{mod.rs,lld.rs,platform.rs}` 和
  `src/cli/commands.rs`，提取普通 build 共用的 `VerifiedNativeBuild`、canonical object graph/link recipe
  与 role-tagged packaging 结果；不复制 linker pipeline。
- `compile_tune_trial(pre_state, checked_plan, request)` 返回 trial + `ArtifactIdentity`；
  `verify_tune_decision_with_source(decision, request)` 完整重算 frontier/compile set、在隔离 cache 重建每个
  trial、执行 size/finalist 状态约束并重放 selected plan。
- 新增 `tests/tune/{trial.rs,replay.rs}`，扩展 `tests/native/{object.rs,artifacts.rs,pgo_o3.rs}`。

## TDD 顺序

1. 写 typestate compile-fail/runtime RED：trial 没有 public publish/ordinary-cache API；任何企图把 trial bytes
   交给普通输出 transaction 的内部适配均不存在，只有 stage-for-measurement 与 checked replay 路径。
2. 写 artifact identity RED：executable/dynamic、header/import role、actual primary size/content、object graph、
   link recipe 对同 request 稳定；destination-only packaging 字节不污染 chosen-code identity。
3. 写 complete trial-set RED：从 Frontier 重算 compile selection，要求 trials plan-digest 排序且一一对应；
   isolated rebuild 对 plan/object/link/content/bytes 任一篡改失败。
4. 写 size/finalist RED：checked u128 `trial*100 <= baseline*110`，所有超限 exact size-rejected；其余按
   actual bytes 替换 KIR rank key 再 diversity truncate，未测/待测集合不能伪造。
5. 写 final replay RED：selected/baseline plan 从 pre-tune state 独立构建，object graph/link recipe 与测量候选
   一致；合法 plan 的 compile/verify/replay mismatch 是 compiler error，不转成搜索 rejection。
6. 运行 `cargo test --test tune trial_ -- --nocapture`、`replay_`、相关 Native tests 与 `cargo test --locked`。

## 实现边界

- trial 不进入 production output 或普通 CKCOBJ cache；阶段 08 的 compile cache 也只返回同 typestate。
- final artifact 不含 runner、tuning symbol、dispatch runtime 或新 runtime dependency。
- source-free inspect 不声称完成 source-aware equalities；只有 tune-use/acceptance 调用该 checker。

