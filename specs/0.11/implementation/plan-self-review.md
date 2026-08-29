# CK 0.11 实施计划自审

## 审查方法

在完成 11 个 task/acceptance 阶段与总验收后，按以下失败模式重新检查：

1. 阶段是否能按现有模块依赖顺序编译，而非形成 frontend ↔ optimizer 环；
2. 是否有验收命令因 Rust integration module qualification 错误而实际运行 0 tests；
3. 是否每个实现项都有先失败测试、明确的禁止项和结构断言；
4. 是否遗漏 checked 首错、print、strict f64、ABI、artifact transaction 或六 host；
5. 是否把 0.12+ 功能、发布/合并权限或降低性能门槛混入计划。

## 发现与修订

### P01：effect solver 最初落点会造成分层反转

初稿把 SCC effect solver 只放在 `src/optimizer/analysis`，同时要求 `check()` 直接使用它
产生 CK2016。这会让 frontend 反向依赖 optimizer，或迫使实现复制一套 source summary。

已修订阶段 04：canonical lattice、parameter mapping 和 SCC solver 放在
`src/ir/effects.rs`，typed source/KIR 仅实现输入 adapter；frontend 和 optimizer 都依赖
共享层，不互相依赖。

### P02：两条测试过滤命令可能运行 0 tests

阶段 02 的 `control_void_slice` 是 module 文件名而不是稳定 test prefix；阶段 08 的两个
`--exact` 命令缺少 integration module qualification。

已修订为阶段 02 运行完整 backend driver，阶段 08 使用
`commands::...` 与 `bench::...` 完整 test name。执行 acceptance 时仍必须检查 test count
大于零，不能只看 exit 0。

### P03：sanitized ownership 命令没有真正启用 sanitizer

初稿只换了 target-dir，没有设置编译/link sanitizer flags。

已同步现有 CI 的 ASan/UBSan `CXXFLAGS`、`RUSTFLAGS`、`ASAN_OPTIONS`、
`UBSAN_OPTIONS`；该 gate 现在检查真实 sanitized binary。

### P04：runtime 文件路径不符合仓库布局

初稿写成不存在的 `native/runtime/src/contract.c`。已改为现有布局下的
`native/runtime/common/contract.c`，并继续要求更新 header/provenance/runtime ABI。

## 通过项

- 01–11 每阶段均有 task 与 acceptance，依赖单向且无跳步。
- 每个行为阶段都明确 red test 在实现前执行；proof、guard、alias、sanitizer、fact audit
  均含故障注入或近邻反例。
- C/WASM/Native、CLI、runtime/cache、文档/compat/version、生成测试、四项性能门槛和六
  host CI 均有责任阶段与总验收映射。
- 最终边界明确允许为验收推送 feature branch，但禁止 PR、main merge、tag 与 Release。
- 0.12+ 项目始终排除；没有以调整 corpus、ignore 或阈值作为失败恢复手段。

## 自审结论

**通过：修订 P01–P04 后，计划无阻断、无明显模块冲突、无空验收阶段。**
