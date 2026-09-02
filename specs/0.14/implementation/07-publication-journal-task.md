# 阶段 07 任务：overlap-closed journal 与 primary-last 发布

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

实现 tuning output set 的 canonical destination、persistent locks、CKTJNL01 schema 1、atomic journal update、
durability barriers、穷举 recovery 与 journal-free orphan 规则，替代 tuning 路径上的 best-effort rollback。

## 仓库落点与接口

- 新建 `src/tune/publication/{mod.rs,destination.rs,lock.rs,journal.rs,recovery.rs,platform.rs}`。
- `TuneOutputSet::resolve(NativeArtifactPaths, decision_path, protected_inputs)` 要求同 canonical parent、合法
  ASCII leaf、无 alias/short-name/device collision，并形成完整 DestinationKey/OutputSetMaterial。
- `PublicationSet::acquire_and_recover` 计算 journal overlap closure、按 destination id 排序持锁并 rescan；
  `publish_verified(decision, role_bytes)` 只接受阶段 06/04 verified output set。
- 新增 `tests/tune/{publication.rs,journal.rs,recovery.rs}`，用 injected phase/barrier crashes 穷举每个状态，
  平台真实 directory flush/atomic no-replace capability 由 Native host tests 覆盖。

## TDD 顺序

1. 写 destination RED：leaf grammar/reserved prefix/device/trailing dot-space、same parent、source/manifest/
   runner/profile/input/output alias、directory case mode、existing handle/Windows long+short identity全部 fail-closed。
2. 写 lock RED：64-hex destination/set names、CKTLCK01 full id、private initializer→flush→atomic no-replace→
   dir flush、owner/no-follow regular validation、persistent final lock 与 reverse release。
3. 写 overlap RED：从 intended set 递归吸收所有相交 valid active/update journals，lock 后 rescan；新增/改变
   closure 释放重试，malformed reserved journal 保留并失败，nonoverlap 不互阻。
4. 写 journal codec RED：exact CKTJNL01 bytes、role layout/order、generation=phase 或 rollback phase+1、path/id/
   basename/digest/size/full set rederive；128 KiB、malformed/trailing/wrong tx 全拒绝。
5. 写 atomic-update RED：private write flush+reopen validate→`.journal.new` no-replace+dir flush→replace active+
   dir flush；attachment 的 active/update/private exhaustive table 每行都有 positive/negative test。
6. 写 publication crash RED：stage flush、Prepared、backup、decision、header/import、primary、Committed 的每个
   rename/barrier 边界注入 crash；primary 始终最后，pair consumer 只接受 decision/output identity 完整匹配。
7. 写 recovery RED：direction/phase/primary identity 穷举选择 rollback/roll-forward，old==new 特例、每个角色
   idempotent；第三种 digest、missing sole copy、journal-free backup 保留证据 fail-closed。
8. 运行 journal/publication/recovery tests、Native all-features，并在当前 host 执行真实 killed-session probe。

## 实现边界

- ordinary `OutputTransaction` 保持原行为；只有 tune output set 使用本协议。
- 平台缺少规范要求的 atomic no-replace/replace/write-through/directory flush 时 tune build 失败，不降级。
- simultaneous multi-file visibility 不作承诺；decision 与全部 role digest 是 pair acceptance 条件。

