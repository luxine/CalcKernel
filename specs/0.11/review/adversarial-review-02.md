# CK 0.11 事实驱动优化器：第二轮对抗性审查

## 输入

本轮重新审查修订后的中英文规范，并以第一轮 B01–B08 为定向回归集，同时主动寻
找修订引入的新冲突。审查仍以语义闭环、实现可行性和验收可判定性为限，不评价命
名偏好或未来版本功能。

## 阻断项回归

| 原阻断项 | 修订后闭环 | 判定 |
| --- | --- | --- |
| B01 KIR 模式/consumer | 明确了 mode-neutral MIR、consumer roots、先裁剪后构造、mode-specific guard、inspection roots 及 unsupported mode 的拒绝时点。现有每条 CLI/backend 路径都能迁移到同一入口。 | 已消除 |
| B02 effects 上限 | 明确只约束外部可达内存，local/private 不计，无法映射访问归 `all`，print/may-fail/unsafe 独立推导。`CK2016` 与 backend memory attribute 都有唯一依据。 | 已消除 |
| B03 pairwise noalias | pairwise fact 只允许 access-scoped metadata；参数级 LLVM/C 承诺要求覆盖全部相关 roots 与 capture/return 规则，第三 root 反例不再成立。 | 已消除 |
| B04 verifier 独立性 | 增加封闭 certificate、局部 derivation/invariant 检查，并明确 analysis output 与 preservation claim 在检查前不可信；不是同源重新查询。 | 已消除 |
| B05 契约事实作用域 | 每个 call edge 独立实例化、实参替换、dominance 与 inline clone 范围均已定义，递归边不复用事实。 | 已消除 |
| B06 LLVM 审计边界 | 固定在 lowering + structural verify 后、LLVM optimization 前，并区分 CK-owned 与 LLVM-owned strengthening。 | 已消除 |
| B07 unsafe main | 已以 `CK2014` 禁止，并明确不形成 executable entry，与现有 main 规则一致。 | 已消除 |
| B08 sanitizer 数学 | 要求 exact/overflow-safe evaluator、checked address interval、wrap/overflow 统一违反路径，并加入极值验收。 | 已消除 |

## 全文重新攻击

### 语义一致性

- 正常模式的契约违反仍是立即 UB，sanitizer 没有被写成正式语义或库 ABI。
- checked failure、runtime print 与 contract memory ceiling 分层，不能借 effects 声明
  消除有序效果。
- 语义 MIR 保持公开文本稳定，KIR 承担 mode-specific guard，不要求在两个 IR 或三
  个 backend 重复实现证明。
- unsafe inlining、递归、sub-slice region 和 alias partition 的事实作用域能由
  dominance/Memory SSA certificate 检查。

未发现新的语义冲突。

### Backend 与 ABI

- C、WASM、LLVM 都消费同一 consumer-specific verified KIR；WASM 仍可在 KIR 前拒绝
  当前不支持的 checked 组合，不会虚构 ABI。
- pairwise metadata 与 parameter-wide attribute 分开，C `restrict` 和 LLVM
  `noalias` 不再被过度承诺。
- exported unsafe header 注释不改变 ABI，并在验收中要求 slice 展平映射。
- LLVM fact audit 不会误管 LLVM optimization 自己推导的合法属性。

未发现新的 ABI 或 lowering 阻断。

### 可实现性与可验收性

- 当前 `CheckedProgram` 可扩展携带 contract；当前 MIR 可作为稳定输入；KIR 可以在
  `src/ir/` 与新优化器模块中并行建立，再一次性切换 backend，符合 shadow 迁移限制。
- 现有六 target CI matrix、C/WASM/Native differential harness、性能 harness、输出
  transaction 与 native LLVM verify/optimize 分界均可复用。
- proof mutation、事实 audit、sanitizer 极值、hot-loop guard 和 pinned baseline 均
  有结构化 pass/fail 条件；没有依赖主观检查的门槛。
- 0.12+ 范围仍被明确排除，修订没有引入 SIMD、specialization、PGO 或 Auto-Tuning。

实现规模大，但不存在无法拆解或需要重新做产品决策的阻断。

## 第二轮结论

**通过：0 个阻断项，0 个明显未闭环隐患。**

第一轮四个非阻断观察项必须进入实施计划和最终验收，但不再要求修改规范。可以进
入第 5 步，按仓库模块与既有测试责任拆分总控、阶段执行、阶段验收和总验收文档。
