# 实施期设计复诊 10：最终 v0.13 基线逐差异集成

## 阻断复诊

总验收要求最终 accepted v0.13 revision 被集成或完成逐差异审计，但 v0.14 分支从较早候选
`94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05` 开始，未包含最终修订
`c44b99cc1954a3ca133cf03c281d0590ce320edb` 的累计修复闭包。旧 replay pin 与若干
跨平台 correctness/performance 修复也因此缺失。完整复现和 RED 证据见
`specs/0.14/review/implementation-blocker-10.md`。

## 决议

- 以最终 v0.13 SHA 为 v0.14 accepted base 与历史 schema-8 replay 身份；exact v0.12 replay
  同步固定到 `1009bae18d1a1ebd37ee9ee095cab9a965e69df8`。
- 吸收 Windows DLL 卸载、checked success-first/cold branch、完整 unsigned SIMD cast、x86
  Native LLVM reduction fallback、短 SLP 三通道条件化及其动态/结构回归。
- v0.14 已实现的 durable profile runtime 明确取代旧 Darwin inode64 导入修补，不把
  `_fstat$INODE64` 加回冻结系统符号面；Linux warning-clean hex 实现继续保留。
- v0.14 新增的 predicated-update 差分测试纳入同一 Windows DLL 生命周期契约。
- 更新 0.12/0.13/0.14 规范、双语 current docs、replay manifest、collector/checker 与 native
  assertions，使源码行为、证据身份和文档闭环一致。

本修订不改变 CK 语言、KIR 3、Bridge ABI 4、Native ABI 1、Runtime ABI 2、调优 wire schema、
安全语义、性能 corpus、样本数、统计方法、性能门槛或十作业 CI 拓扑。任何本轮代码提交都会使
旧 exact-SHA CI 失效，必须对新 SHA 完整验收。
