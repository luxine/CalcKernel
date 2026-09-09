# 阶段 03 任务：generation pipeline、collector 与 flush 生命周期

## 目标

实现真正可执行的 Native profile generation：固定版本 pipeline、relaxed/saturating atomic counter、
安全目录锚定、transactional raw shard publication、executable normal-return flush、library 显式 sticky
flush，以及 `ckc pgo build` 的一次真实训练流程。异常路径不能留下 final artifact 或损坏 completed shard。

## 仓库落点

- 新建 `native/profile_runtime/` 的 private header/common 与 Linux/Darwin/Windows platform 实现，
  修改 `scripts/bootstrap-llvm.{sh,ps1}`、`build.rs` 和 prefix manifest validation；profile runtime
  object/hash 与既有五个 ordinary runtime objects 分开。
- 修改 `src/backend/llvm/{kir_lower.rs,entry.rs,notices.rs}`、Native artifact/linker/header export 路径，
  生成 counter storage/site table/path identity、entry-wrapper flush 与 64-hex library flush symbol。
- 修改 `src/cli/{pgo.rs,commands.rs,output.rs}`，让 temporary artifact、child、shard、final profile、
  final executable 使用同一最终 output transaction；generation bypass cache。
- 新建 `tests/native/profile_generation.rs`、platform/runtime fixtures，扩展 CLI/artifact/header/
  bootstrap/release audit contract tests。

## TDD 顺序

1. 写 runtime ABI RED：profile runtime 独立 version/digest；ordinary/profile-use artifact 不链接它，
   generation artifact 只暴露 library flush control entry且不改变 Native ABI 1/Runtime ABI 2。
2. 写 counter RED：目标无 lock-free atomic64 时 build 拒绝；relaxed update、wrap->saturated/overflow、
   histogram/constant bounded index、multi-thread/multi-process 无共享写文件。
3. 写 directory RED：build 时绝对化并逐组件拒绝 symlink/reparse，捕获 file identity；runtime 重新
   no-follow 打开并锚定 temp/fsync/validate/no-replace rename，目录替换或不稳定身份拒绝。
4. 写 executable RED：normal `main` return 写一个 completed shard；nonzero/signal/abnormal/missing/
   empty/write-failure 使 `pgo build` 原子失败，prior outputs 保留。
5. 写 library RED：header/export 是 `ck_profile_flush_<64-lower-hex>() -> i32`；host quiesced first flush
   恰写一个 shard，并发/repeat 返回相同 sticky status；unload/`DllMain` 不 I/O。
6. 写 event-count differential RED：early return、break/continue、checked failure、recursion、loop/
   slice/constant 对应 exact raw counters/equations；temporary file 被 merge 忽略但计数。
7. 写 code-shape RED：compiler-private initialization guard 带 Native `NoInline` attribute；LLVM
   不得把完整 runtime initialization 参数准备复制进被内联 function 或 loop site。
8. 实现 collector/platform adapter、generated support data、entry/header/link integration；最后实现
   `ckc pgo build` 临时目录生命周期、child execution、merge/use handoff和 rollback。

## 实现边界

- runtime 不依赖 LLVM profile runtime、compiler shared library 或新非系统动态库，不联网。
- flush 前 host quiescence 是 temporary API precondition；实现不得用 concurrent counter copy 假装安全。
- generation 用 baseline implementation；`--cpu multiversion` 只绑定 target-set identity，不训练 dispatch。
- 此阶段 final `pgo build` 可以先以未加权普通 O3 use 骨架产物验证生命周期；真正 PGO application
  在阶段 04 接入，测试必须明确区分。

## RED/GREEN 证据

记录 Native toolchain manifest、profile runtime object/hash、每种生命周期 RED、shard digest 与 CLI
transaction 结果到 `target/acceptance/v0.13/stage-03/`。
