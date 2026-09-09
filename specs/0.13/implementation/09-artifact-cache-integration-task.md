# 阶段 09 任务：named-object artifact、CKCOBJ03/cache 4 与 CLI 原子集成

## 目标

把 baseline、variants、dispatcher 与 private support 作为已审计 named objects 装配成 executable/
dynamic/static final artifact；升级 Native cache 为 `CKCOBJ03` key/manifest schema 4，完整绑定 profile/
target-set/variant/dispatch/runtime/physical artifact identity；最终 CLI output set 原子提交且无 profile
runtime 泄漏。

## 仓库落点

- 修改 `src/backend/artifact/{archive.rs,lld.rs,platform.rs,mod.rs}`，让 linker/archive 接受稳定命名
  object list；static member 与 Windows import library/export table deterministic。
- 修改 `src/cli/cache/{key.rs,entry.rs,store.rs,mod.rs}`，新增 bundle manifest/reference validation、
  per-variant object cache 与 schema migration rejection。
- 重构 `src/cli/commands.rs` build pipeline 与 `output.rs` transaction，统一 ordinary/generate/use/
  multiversion 的 primary/header/import-library/profile/sidecar outputs。
- 新建 `tests/native/multiversion_artifacts.rs`，扩展 cache/artifact/CLI/reproducibility/contracts tests。

## TDD 顺序

1. 写 assembler RED：named object order/role/digest/symbol table closed；executable/dynamic/static 成功，
   multiversion object 在 lowering/link/output 前拒绝；single-version baseline/native use object 保持支持。
2. 写 archive/link RED：separate modules 无 cross-variant LTO/partial-link 偷换；static members 唯一稳定，
   runtime symbol namespaced；dynamic/import/export 只含 public CK thunks与 generation-only flush（仅 generate）。
3. 写 cache format RED：magic `CKCOBJ03`、key/manifest 4 canonical/checksummed；旧 CKCOBJ02、unknown/
   missing/extra/reordered/redirected object、path escape/symlink/bad digest/manifest mismatch 完整 miss/reject。
4. 写 key RED：包含 0.12 全部字段及 profile mode/contract/exact digest、physical artifact kind/topology
   validation、阈值/cost/site、target set/per-variant proof/codegen、dispatch/detector/runtime 和全部 budget。
5. 写 cache behavior RED：generate 永不 cache；use variant 可 individually hit，但 dispatcher manifest
   或任一 referenced object 不匹配则 complete bundle miss；失败不发布 cache entry。
6. 写 output transaction RED：primary/header/import library/profile/sidecar same-filesystem stage/rollback；
   destination alias/duplicate/symlink/commit injection 保留每个 prior output且清理 debris。
7. 写 final audit RED：ordinary/use/multiversion final artifact 不 import profile writer/LLVM/compiler/new
   shared lib，不含 generation path/counters/flush；public ABI/header byte-stable，hidden symbols/feature-contained。
8. 写 reproducibility RED：相同 source/toolchain/flags/target-set/profile bytes 在不同 cwd/map/cache hit/
   miss/order下产生 byte-identical unsigned artifacts和 manifests。
9. 实现 named-object API、cache entry/bundle store 和 CLI orchestration；先用 fixture GREEN，再跑 real
   lld/archive/object matrix与 disassembly/symbol audits。

## 实现边界

- cache hit 是完整 bundle 原子判断，不能将新 dispatcher 与旧 variant 混装。
- operational generation directory/path 永不进入 final profile/artifact/cache identity。
- 物理 dynamic/static/object kind 进入 artifact/cache key，但不分裂 Native-library profile identity。

## RED/GREEN 证据

记录 cache mutation matrix、hit/miss bundle digests、transaction fault injection、artifact/symbol/import/
reproducibility results 到 `target/acceptance/v0.13/stage-09/`。
