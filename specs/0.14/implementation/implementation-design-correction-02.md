# CK 0.14 实施期设计修订 02：搜索交互与 Schema 9 原始证据闭包

状态：已采用；不降低任何正确性、性能、资源或 CI 门槛。

## 触发事实

真实 `ckc tune build` 暴露了四个在实现前无法由静态计划充分观察的问题：

1. CKTUNE01 的 `Unit.class` 要求单元内所有 site/variant 同类，而设计正文曾要求把跨类别重叠
   全部聚成一个单元；两条要求不能同时成立。
2. LLVM O3 可以在 Late Layout 消费前删除所选 block；直接携带 O3 前名称会把合法候选误报为
   “unknown block”。
3. Schema 9 的编译时间只保留 `Command` 与数值数组，没有把单次耗时和命令绑定，独立 checker
   无法证明 `samplesNs` 来自对应调用。
4. Schema 9 要求 `certificateDigest`，却没有定义其字节编码；同时把事件日志描述成额外的编译器
   输出协议，但冻结 CLI/环境/输出模板没有承载该协议。

这些是规范自身的可实现性/可验证性矛盾，必须修订；不是为了绕过失败门槛。

## 冻结修订

- 调优单元在同一 alternative class 内做 overlap clustering，保持 CKTUNE01 `Unit.class` 和完整
  site-alternative 集合不变。跨类别组合仍由 canonical whole-plan expansion 探索。后置变换令先前
  anchor/precondition 失效时，该 expansion 以 `illegal` disposition 和稳定诊断记录，搜索继续；
  其他编译错误仍 fail-closed。
- 后端尽量保留 layout 选择涉及的函数，并在 LLVM O3 后把所选映射与真实 module 名称复核。
  只有完整存活映射才应用；完全被 O3 消除的映射成为 no-op，不改变 LLVM pipeline，也不凭空恢复
  已删除代码。
- 编译时间的 18 个条目改为闭合 `TimedCommand { command, elapsedNs }`；十五个测量样本必须逐项等于
  对应 receipt。全部原阈值、轮换、缓存隔离和次数保持不变。
- 有证书时，`certificateDigest` 使用证书 tags 1..8 的八个 `DigestBytes`，按规范顺序置于
  `P("CK-V014-TUNE-CERTIFICATE\0", ...)`；基线原因仍为 null。
- `eventLog` 是 direct-child supervisor 在成功退出后，从已解码 decision、闭合 cold/warm outcome
  与 cache 前后态确定性导出的原始 receipt；checker 重新导出计数和顺序。它不是未声明的第二套
  编译器 CLI 输出。
- CacheSnapshot 是锁内时间点 receipt。checker 重哈希两份 receipt 命名的文件，验证摘要、cold-before
  为空、warm-before 等于 cold-after，并只把最终 snapshot 与运行后 live namespace 做完整性比较；
  不把已经被子进程改变的 live 目录错误地当成历史前态。

## 验收影响

- CKTUNE01 wire schema、语言/ABI、安全证明、候选预算和全部性能阈值均未改变。
- `tests/optimizer/tuning.rs` 必须证明跨类别冲突记录为 illegal 且搜索不中止。
- Schema 9 mutation tests 必须篡改 `TimedCommand.elapsedNs`、certificate digest、event/candidate count、
  cache continuity 时 fail-closed。
- 本修订与英文/中文正式设计及 `performance-schema-9.md` 同步；后者仍是性能报告的权威附件。
