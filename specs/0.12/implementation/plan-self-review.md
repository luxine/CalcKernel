# CK 0.12 实施计划自审

日期：2026-09-01
对象：`specs/0.12/implementation/00`、`01`–`10`、`99`
结论：**通过；计划经两轮依赖/可执行性自审后无阻断项。**

## 自审检查表

- [x] 每个设计 deliverable 恰有实施阶段与验收 owner，无遗漏、重复最终 owner 或循环前置。
- [x] 阶段顺序匹配现有模块：profile/KIR schema -> Native bridge -> loop legality -> transaction/
  checker -> specialization -> unroll/SLP -> Loop SIMD/frontier -> release identity/docs ->
  performance/CI。
- [x] 每阶段包含可观察 RED、最小实现、重构与非零 test filter；没有“直接实现后补测试”。
- [x] Checker independence、transaction rollback、audit non-refund、checked/strict-f64/public ABI
  在阶段局部与最终验收都出现。
- [x] C/Wasm/Inspection、Native baseline/native、sanitizer 与六 host 都有明确 owner。
- [x] Schema 7 的 12-channel/3-channel、0.11 replay、C/Rust oracle、domain/size/compile-time 和全部
  固定门槛都进入阶段 10 与总验收。
- [x] 阶段 09–10 不创建 tag/Release/merge；远程 CI 只定期查询；exact-SHA self-reference 问题由
  “最终 SHA 后不再回写”解决。
- [x] 计划没有依赖不存在的持久化 KIR cache、未固定的 target、或要求子代理。
- [x] 所有文档路径、test aggregator 与命令能由当前仓库结构承载。

## 发现与修订

第一轮发现并修订：

1. 阶段 01 原把 Native cache schema 与 feature-disabled profile 混在一起；已把 cache v3 移到
   阶段 03，与实际 bridge/Native compile flow 同时验收。
2. 阶段 02 原要求在尚未 bootstrap Native toolchain 时验证 Native exhaustive match；已限定
   本阶段验证 C/Wasm，Native rejection/lowering 在阶段 03 用 all-features 真实覆盖。
3. 原计划把动态 SHA/run id 追加到被测文档，会造成 exact-SHA 自引用；现统一写入 ignored
   `target/acceptance/v0.12/` 与 CI artifact，最终提交后不回写。

第二轮发现并修订：

4. 原顺序在阶段 09 先切 package 0.12、阶段 10 才更新 current docs，会使阶段 09 全仓 CI 被
   repository/docs contract 正确拒绝。现阶段 09 一次完成 0.12 identity/current docs/
   compatibility，阶段 10 才以真实一致的候选运行 schema 7 与 exact-SHA 全矩阵。
5. 阶段 10 acceptance 已补回 default/all-feature/release build、sanitizer 与 artifact/JIT audits，
   避免性能脚本变更后只跑选择性测试。

设计交付映射复核：profile/KIR/cache 分属 01–03；loop/dependence/predicate 属 04；状态分层/
checker 属 05；specialization 属 06；unroll/SLP 属 07；Loop SIMD/frontier/differential 属 08；
version/current compatibility 属 09；全部 performance/size/compile-time/CI 属 10；总契约由 99
再次累计检查。没有循环依赖或无人负责的设计条款。

## 最终判定

计划与当前仓库模块、toolchain bootstrap、TDD 顺序、提交边界和最终 exact-SHA 证据闭环一致，
可以提交文档检查点并进入阶段 01。实施中若出现真实规范反例，仍按总控复诊，不得降低门槛。
