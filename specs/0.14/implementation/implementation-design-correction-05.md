# 实施期设计复诊 05：Schema 9 evidence root 必须形成完整文件闭包

## 阻断复诊

最终 collector/checker 对照审计发现，首版 full collector 会在 evidence root 中留下
compile-time 临时 `.so/.h`、对应 cache、profile-generation 中间产物以及 tuning 发布锁；
这些文件不属于任何 `FileIdentity`。同时 checker 只验证已引用文件存在，没有反向拒绝
未引用文件。这样 report 不能证明它声明的是完整证据集合，也违反性能 schema 已冻结的
“每个 evidence-root 文件都有身份、未知条目无效”要求，属于证据真实性阻断。

## 修订决议

1. compile-time 每次构建后先确认完整动态输出存在，再在计时区间外删除输出、发布锁和
   该次独立 cache；报告只保留规范允许的 `TimedCommand`。
2. v0.13 profile 合并并检查成功后删除 generation library/header、raw shard 和 profile
   cache，只保留最终 `.ckprof`。
3. 普通 artifact build 在记录完整输出身份后删除不参与证据的独立 compiler cache；三次
   tuning session 的 cache 保留，因为其 before/after snapshot 是规范证据。
4. Checker 递归收集 report 中全部 evidence-root `FileIdentity`，要求同一路径身份一致，
   再与 evidence root 的全部真实普通文件做双向精确集合比较；缺失、额外、symlink 或特殊
   条目一律失败。

## 验证与门槛

新增 mutation test：完整 contract closure 通过，注入一个未标识文件必须被拒绝。Full
Linux CI 继续验证真实 replay/cumulative/cache/output 树。该修订只收紧证据闭包，不改变
性能、正确性、资源、身份或 CI 门槛。
