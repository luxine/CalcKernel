# 阶段 08 任务：tune cache、CLI 工作流与普通路径隔离

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

把已验证子系统接入 `ckc tune build|inspect` 和 `ckc build --tune-use`，实现 compile/measurement/
completed-decision 三类私有 cache、warm exact reuse、4 GiB hard limit、interrupt handling 与 fail-closed CLI 矩阵。

## 仓库落点与接口

- 新建 `src/tune/cache/{mod.rs,entry.rs,path.rs,store.rs,evict.rs}`，namespace 固定现有 cache root 下
  `tune-v1`；compile key、measurement key、completed decision key 使用不同 domain 与完整 identity。
- 新建 `src/cli/tune.rs`，修改 `src/cli/{mod.rs,args.rs,commands.rs,run.rs}`；解析 `tune build/inspect`、
  `--config/--budget/--tune-out/--no-tune-cache/--tune-use`，所有 invalid combination 在输出/runner 前失败。
- 提取共享 Native request/output API，普通 `build` 仅在显式 `--tune-use` 时进入 source-aware replay；
  `run/emit-kir/build-llvm` 拒绝 tune-use。
- 扩展现有 `src/cli/cache` 到 CKCOBJ04/schema 5 并让 `cache clean` 安全清理 ordinary+tune namespaces；
  新增 `tests/tune/cache.rs`、`tests/cli/tune.rs`，扩展 native cache/artifact/CLI tests。

## TDD 顺序

1. 写 CLI RED：primary command、default standard、kind executable/dynamic、cpu native、O3、host target、
   optional pgo-use/modes；static/object/baseline/multiversion/generate/sanitizer/跨 target/重复未知参数拒绝。
2. 写 destination/side-effect RED：default `.cktune`、explicit same parent、完整 NativeArtifactPaths；所有
   invalid CLI/manifest/identity 在创建输出、cache entry 或运行 harness 前失败。
3. 写 cache format RED：private permission/no-follow/checked bound/digest/atomic publication/corruption miss/
   traversal/owner failure；local 32-byte CSPRNG salt 的 raw bytes 不进入 decision，digest 可重导。
4. 写 interrupted cache RED：compile 只存 fully verified nonpublishable trial；partial measurement 整 phase 丢弃
   row 0 重启，禁止 session sample splice；timeout/crash/bad digest/partial decision 不作 successful result。
5. 写 cold/warm RED：两个 distinct empty namespaces 的 cold run measurement-independent choice/plan/object/
   link/published bytes 相同；warm 从 first post-inventory 原样开始，compile/measure count 都为 0 且 decision/
   output bytes exact；`--no-tune-cache` 强制 fresh session。
6. 写 tune-use RED：source/compiler/schema/CPU/features/profile/mode/kind/frontier/plan 任一 mismatch 失败，无
   fallback；不同 destination 可重新 packaging，但 object/link 仍 exact；baseline decision 重放 empty plan。
7. 写 ordinary isolation RED：无 tune command/use 时对 cache filesystem 做 access audit，要求 0 tune read/write、
   0 harness process，ordinary optimized object 与基线一致；`cache clean` 同时安全清理两 namespace。
8. 运行 tune CLI/cache/session/native tests，再执行全量 all-features、artifact audit 与 sanitizer tests。

## 实现边界

- completed baseline decision 含 timeout 时不能 warm reuse；其他完整 baseline reason 可以 exact reuse。
- cache LRU 是 deterministic、hard 4 GiB；锁定 session inventory 后任何非声明 access 都是验收失败。
- CLI 不新增隐式训练、run tuning、emit-kir tune-use 或静态/object tune output。

