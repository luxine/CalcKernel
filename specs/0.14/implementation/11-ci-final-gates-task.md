# 阶段 11 任务：exact-SHA 十作业 CI 与最终门禁

> **当前定位：已落地的十作业基础。** 本阶段不能单独推送或签署本轮候选；
> 阶段 19 负责把阶段 12–18 接入同一拓扑并执行唯一最终验收。

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

在不增加或减少 required 拓扑的前提下，把 v0.14 parser/planner/process/cache/journal/replay/schema9 验收接入
现有 quality、native integration、六 native host、两 stable performance job，并以同一最终 SHA 完成本地总验收
和远程 exact-SHA 验收。

## 仓库落点与接口

- 修改 `.github/workflows/ci.yml`，保持十 job：quality、native-integration、Linux/macOS/Windows × x86-64/
  AArch64 六 host、Linux enhanced x86-64/AArch64 两 performance；全部绑定 dispatch/ref 的 resolved SHA。
- 扩展 `tests/contracts/ci.rs`、Native platform tests 与 fuzz/mutation harness；必要的 ASan/sanitizer runner 放
  `scripts/`，不得进入 release dependency。
- performance artifact 上传完整 `target/ckc-perf/`、exact v0.13 replay bundle、schema8 cumulative closure、
  schema9 decision/output/receipt/archive evidence；generated evidence 不回写 candidate commit。
- 最终执行 `specs/0.14/implementation/99-final-acceptance.md` 并把静态记录保存在 ignored
  `target/acceptance/v0.14/final/`。

## TDD 顺序

1. 写 CI topology RED：exact 十 job、六 host/两 tier、无 `continue-on-error`、required capability 不可 skip，
   checkout/build/test/report 全部验证同一 candidate SHA。
2. 写 quality RED：feature-disabled tests、fmt/clippy/doc、manifest/decision/journal/cache mutation、docs/contracts、
   fuzz-style bounded inputs、TypeScript oracle 固定 provenance 全覆盖。
3. 写 native-integration RED：真实 executable/dynamic tune build、inspect、tune-use、wrong-identity negative、cold/
   warm cache、killed-session recovery、no-splice、artifact dependency audit，不能用 mock linker 代替。
4. 写 six-host RED：runner argv/env/process containment、filesystem alias/short-name、atomic primitives/directory flush、
   journal every-phase recovery、ABI/output set、ordinary isolation；平台能力缺失 hard fail。
5. 写 performance RED：两个 job 先验证 x86-64-v4/SVE2，再执行 historical schema8、fresh cumulative schema8、
   full schema9 collector+checker；candidate/report/archive/CI head SHA 相等。
6. 在 clean worktree 执行所有阶段 acceptance 的本地并集与 final checklist；任何变化后重跑受影响门禁并
   形成 final implementation commit。
7. 推送 `design/v0.14-offline-autotuning`，显式触发 `ci.yml`，记录 run/head SHA；每次远程查询间隔至少
   60 秒，可继续不依赖结果的本地审计，不能原地高频等待。
8. 所有十 job 对同一最终 SHA 成功后验证 artifact/report digests；保持 branch worktree clean，不合并 main，
   等待用户审查。

## 实现边界

- required job/capability/test 不得通过 condition、optional input、continue-on-error 或空 artifact 绕过。
- remote transient failure 可重跑同 SHA；产品/checker/fixture 变化必须新 SHA 全量重跑。
- v0.13 accepted-base 若仍未满足，明确保持 release blocked；不得伪造 tag 或降低 v0.14 本身门禁。
