# 实施阻断复诊 10：最终 v0.13 基线与跨平台性能修复闭包

## 复现

最终总验收审计发现，v0.14 起点包含 v0.13 候选
`94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`，但最终接纳的 v0.13 修订是
`d5a2491672477634070b0c36b77cb8ad4bf7df56`。原复诊时 `git cherry` 证明修复提交
`f6efbef`、`930ee5f`、`9ce0f3d` 未作为祖先进入 v0.14；原 v0.14 性能 replay 也仍
固定旧 v0.13 与旧 v0.12 候选。因此“最终 accepted v0.13 revision 已集成或完成逐差异
审计”的总验收项没有证据支持。

逐文件复诊进一步复现以下有效缺陷：

- x86 KIR 在每个 chunk 提前执行 horizontal reduction，阻断 LLVM 使用循环累加器；
- checked guard 把失败块排在成功块之前，且没有 `1:2000` 冷失败分支权重；
- vector 与新增 predicated-update 差分测试在 Windows DLL 仍加载时删除目录；
- x86 Rust oracle 用有符号 `_mm_cvtepi32_pd` 实现 CK `u32 -> f64`，高位域错误；
- 四元素 `slp_quad` 的每个 timed sample 前缺少三个 channel 完全相同的未计时条件化；
- benchmark/native assertions 不认识显式的 x86 Native LLVM reduction fallback；
- v0.12 replay 仍固定已被后续 CI 否定的 `1c2596da...`，v0.13 replay 仍固定
  `94aad2d6...`。

## 判定

阻断成立。它同时影响跨平台 correctness、性能真实性与历史基线身份，不能由旧的本地或
远程成功结果代签，也不能通过降低阈值、减少 corpus 或放宽 required job 解决。

## 修订边界

- 逐文件吸收最终 v0.13 三提交的有效语义；不要求制造无意义的 ancestry，只要求实现、测试、
  规范与 replay identity 等价且可独立审计。
- v0.14 已有的 durable profile publication 取代 v0.13 Darwin `_fstat$INODE64` 导入方案：
  最终产物明确禁止该未冻结符号，同时保留并强化目录身份、create-new、sync 与 no-follow 门禁。
- 使用现有显式 weighted-branch bridge API，不恢复依赖 block 名称的 C++ 隐式规则。
- x86 reduction 只交回 Native LLVM loop vectorizer；AArch64 固定向量 KIR reduction 不变，
  object disassembly 仍必须单独证明 x86 SIMD。
- 完整 `u32` oracle 语义、短 kernel 条件化、样本数、rotation、统计量、90%/95% 等阈值和十作业
  拓扑保持不变。
- v0.12 replay 固定 `1009bae18d1a1ebd37ee9ee095cab9a965e69df8`；v0.13 replay 固定
  `d5a2491672477634070b0c36b77cb8ad4bf7df56`，并同步重算各自 manifest digest。

## TDD 证据

1. x86 reduction 回归测试先观察到四个错误 KIR candidate，修复后只留下稳定 fallback。
2. Native LLVM 测试先观察到 failure-first 且无 `!prof`，修复后 success-first 并带
   `branch_weights 1, 2000`。
3. 两个 DLL 生命周期契约分别先因缺少 `drop(o3)`/`drop(o0)` 失败，再在 cleanup 前显式卸载后通过。
4. oracle 源契约先发现 `_mm_cvtepi32_pd`，动态审计输入随后加入 `i32::MAX + 1` 与 `u32::MAX`。
5. manifest 与 runner 契约分别先因缺少 short-kernel conditioning 失败，再在每个 timed
   sample 前对同一 runner 执行一个未计时 batch 后通过。
6. schema-8/schema-9 pin 契约先拒绝新的 exact SHA，再在全部 owner 与 digest 同步后通过。

本轮只修复真实反例并增加不可回退测试；最终结论仍须由新 candidate SHA 的完整本地命令和十作业
远程 CI 支持。
