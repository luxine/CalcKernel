# CalcKernel 路线图

[English](../../project/roadmap.md)

本文档非规范，只列出尚未交付的可能工作，不覆盖 [0.14 兼容策略](compatibility.md)。

- 评估 indirect calls 与 indirect-call promotion，且不能削弱闭合 effect、ABI 或
  profile-mapping contract。
- scalable KIR vector 与 0.14 fixed-width target variant 分开评估。
- adaptive JIT PGO 单独评估；0.14 profile 与 tuning decision 保持 offline and immutable。
- Source SIMD type 与更丰富的 target-specific vector facility 只通过独立评审的未来 contract 加入。
- 强化 target-specific LLVM calling convention 与 data-layout reporting。
- 只以未来明确 versioned ABI addition 的方式评估 WASM checked bounds/status。
- 改进 debug/source mapping 与 artifact introspection。
- 增加 conformance fixture、fuzzing 与可复现性能历史。
- 定义未来 1.0 language/ABI commitment 的准入要求。
- Cross-compilation、program argument、更丰富 I/O 与 public JIT API 仅通过独立 versioned
  design 评估。

任何条目在进入 versioned design 与 release contract 前都没有交付日期或兼容效力。
