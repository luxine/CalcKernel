# 实施阻断复诊 12：x86 归约识别污染生产 IR

## 复现与判定

V0.12 新 exact-SHA run 在 x86-64 的 `scalar unchecked/integer_accumulate` 上稳定超过 V0.11
replay 约 14.86%，违反未修改的 8% 退化上限。二十个样本高度聚合，同行 Clang 校准一致，阻断
成立。保留的反汇编显示候选多出逐轮 `or`/increment 递推。

根因是 V0.14 继承的 x86 integer-memory-reduction handoff：它为了识别目标 loop，在判定前对每个
O3 production function 原地执行 `mem2reg`。即使 `integer_accumulate` 不是目标 memory reduction，
标准 LLVM O3 的输入也已被提前改写。

## 修订闭包

- 在 detached function clone 上执行临时 `mem2reg` 与 loop 分类；
- 只把已证明目标 loop 的 `llvm.loop.interleave.count = 8` metadata 映射回 production loop；
- 无 non-local load 的函数在 clone 前跳过；
- 契约测试要求 clone 隔离，并禁止旧的 production-module promotion；
- V0.13 exact replay pin 前移到包含同一修复的
  `c44b99cc1954a3ca133cf03c281d0590ce320edb`，manifest digest 同步重算。

归约策略、interleave 宽度、语言/ABI、安全语义、性能 corpus、样本与阈值均未改变。动态结论仍由
新 SHA 的 x86-64 远程性能作业签署。
