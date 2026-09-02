# 阶段 09 任务：0.14 identity、兼容契约与 current docs

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

把实现声明为一致的 0.14.0 候选：升级 CKCOBJ04/key+manifest schema 5，保持语言/KIR/Native/Runtime/
bridge/profile/multiversion ABI 不变，冻结 v0.13 clean-miss 与 `.cktune` 私有兼容边界，并同步所有双语
current 文档、CLI help、release audit 和 compatibility fixtures。

## 仓库落点与接口

- 修改 `Cargo.toml`/`Cargo.lock`、`src/cli/cache/{entry,key}.rs`、`src/tune/schema.rs`、版本输出及
  build-time identity；版本为 `0.14.0`，ordinary cache 为 `CKCOBJ04`、key/manifest 5。
- 新增 `tests/fixtures/compatibility/v0_14/manifest.toml`，扩展 `tests/{compatibility.rs,contracts.rs}`、
  `tests/contracts/{docs,release,repository,native_toolchain}.rs`。
- 同步英文及 `docs/zh-CN/` 中 `index`、`compiler/{architecture,optimizer}`、`guides/performance`、
  `reference/{cli,diagnostics}`、`project/{compatibility,roadmap,release,release-checklist}`；更新 README 与
  CHANGELOG 双语当前入口，保持正式规范不引用阶段计划或审查历史。
- 更新 `scripts/audit-ckc-release.{sh,ps1}`、`scripts/audit-native-artifact.{sh,ps1}` 与 notices/provenance
  仅当真实依赖或归档内容变化要求。

## TDD 顺序

1. 写 version/schema RED：Cargo/CLI/docs/checker 必须同时为 0.14.0；Language unchanged、Native ABI 1、
   Runtime ABI 2、KIR 3、bridge 4、profile/multiversion 1，tune schemas 1，CKCOBJ04/cache 5。
2. 写 compatibility RED：CKCOBJ03/schema4 只能 clean miss，不升级解释；v0.13 `.ckprof` 因 compiler/source/
   cache identity 不匹配而拒绝；v0.14 `.cktune` 对 future schema/unknown field fail-closed。
3. 写 default-behavior RED：0.13 language/source/diagnostic/strict-f64/checked-first-error/effect/slice/public ABI/
   multiversion 全部 fixture 继续通过；普通 build 不出现 tune state。
4. 写 docs contract RED：双语相同相对路径、命令/默认值/precondition/runner security/cache/replay/diagnostic/
   ABI/zero-dependency 表述一致；所有 `Rust_CalcKernel` repository identity 替换为 `CalcKernel`。
5. 写 release audit RED：分发编译器包含 LICENSE/THIRD_PARTY_NOTICES，artifact 无 runner/tuning symbol/new shared
   dependency；归档审计可重算 compiler/LLVM/runtime/schema identity。
6. 运行 compatibility/contracts/docs/release tests、fmt/clippy/doc、feature-disabled/all-features 全量测试和双平台
   release audit 脚本可用分支。

## 实现边界

- 不创建 tag/Release，不把 Proposed 设计状态伪装成 accepted release。
- 正式 `docs/` 只保留当前用户/维护者文档；实施计划仍留在用户明确要求的 `specs/0.14/implementation/`。
- 不更改 source syntax、C/WASM contract、public Native ABI 或 final runtime dependency。

