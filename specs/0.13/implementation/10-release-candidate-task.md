# 阶段 10 任务：0.13.0 identity、兼容契约与 current docs

## 目标

把已通过功能/本地 artifact 验收的实现声明为 0.13.0 release candidate：升级 package/lock/verbose
identity 和私有 schema/ABI 常量，保留 Native ABI 1/Runtime ABI 2，更新英中 current docs、CLI、
compatibility/release/audit contract，且不误报尚未通过的性能或正式 Release。

## 仓库落点

- 修改 `Cargo.toml`、`Cargo.lock`、`CHANGELOG*.md`、`README*.md`。
- 修改 `src/backend/llvm/{ffi.rs,notices.rs}`、KIR/profile/target/proof/audit/private runtime/cache/
  performance schema 常量及 `--version --verbose` 输出。
- 更新 `docs/{reference/cli.md,compiler/{architecture,optimizer}.md,guides/performance.md,
  abi/{llvm,modes}.md,project/{compatibility,roadmap,release,release-checklist}.md}` 与完整 `docs/zh-CN`
  镜像；必要时更新 getting-started/backend-selection/diagnostics。
- 更新 `tests/contracts/{release.rs,docs.rs,native_toolchain.rs,repository.rs}`、compatibility fixtures、
  audit scripts与 release workflow identity assertions。

## TDD 顺序

1. 写 release identity RED：Cargo/lock/`ckc --version`/verbose/README/changelog/current docs 全部要求
   0.13.0；LLVM 22.1.8、bridge ABI 4、KIR 3、CKCOBJ03/key+manifest 4、Native 1、Runtime 2 一致。
2. 写 compatibility RED：0.12 profile/cache/bridge/KIR private schema fail-closed，0.12 source/
   diagnostics/public Native ABI/runtime/artifact behavior保持；0.10/0.11/0.12 历史身份仍可追溯。
3. 写 CLI docs RED：ordinary 无训练默认路径、`pgo build/merge/inspect`、explicit library workflow、
   topology/object restrictions、full flush symbol、security/privacy、profile invalidation/error categories 完整。
4. 写 architecture/optimizer RED：profile non-proof、O2 late-layout closed delta、O3 transaction、same
   pre-state variants、target tables、dispatcher/cache/artifact identities与源码实现互相引用。
5. 写 release/audit RED：distributed archive 包含所需 private runtime notices/objects但无运行时外部
   dependency；release workflow仍六平台、pinned action/LLVM manifest、tag/version/SHA/provenance 验证。
6. 写 bilingual parity RED：每个 0.13 normative/current doc 有英中镜像和相同 command/schema/threshold；
   0.14 Auto-Tuning、indirect calls/scalable KIR/JIT adaptive 仍明确 future。
7. 最小升级版本与 constants，迁移 tests/fixtures；随后更新 current docs，运行链接/命令/术语检查，
   最后让 release/native/JIT audits 对真实 release binary GREEN。

## 实现边界

- 不删除历史 changelog/compatibility 记录；pre-release `specs/0.13` 与 current docs 职责明确。
- 不创建 tag、Release 或自动合并；性能/CI 未完成前只能称 candidate。
- Native ABI 1/Runtime ABI 2 不因 private flush/dispatch helper 提升；任何意外 public ABI delta 是阻断。

## RED/GREEN 证据

记录版本 identity RED、schema migration fixtures、英中 parity/audit 与 release binary SHA 到
`target/acceptance/v0.13/stage-10/`。
