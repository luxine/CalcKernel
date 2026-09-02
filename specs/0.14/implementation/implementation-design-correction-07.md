# 实施期设计复诊 07：v0.13 profile inspection 字段与内容必须按 CKPROF01 验证

## 阻断复诊

Full collector 路径审计发现，首版代码把 v0.13 `pgo inspect --json` 的编译器来源
读成不存在的 `identity.compiler.source`；冻结 CKPROF01 inspection schema 实际使用
扁平字段 `identity.compilerSource`。因此七个 profile 中第一个就会被误报为缺失，远程
Schema 9 无法开始测量。Checker 同样使用了错误路径，并且没有显式锁定 profile 的
target/mode/completeness 内容，属于真实运行与证据检查双重阻断。

## 修订决议

Collector 按 `schema=1`、`format=CKPROF01` 和小写 64 位十六进制
`identity.compilerSource` 读取身份。Checker 用 v0.13 retained compiler 独立 inspect
每个 `.ckprof`，除来源摘要外还要求 package `0.13.0`、当前稳定 host target、native
library/O3/native CPU、unchecked bounds/overflow、strict float、非 sanitizer、兼容 package、
至少一次完整 run、至少一个 observed site 且 observation 不完整标志为 false。

## 验证与门槛

新增冻结 inspection fixture 回归：扁平字段成功，伪造回旧嵌套路径必须失败。该修订
恢复既有 profile 真实性检查，不改变 PGO、性能、资源或 CI 门槛。
