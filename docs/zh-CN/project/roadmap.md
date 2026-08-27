# CalcKernel 路线图

[English](../../project/roadmap.md)

本文档非规范，只列出尚未交付的可能工作，不覆盖 [0.10 兼容策略](compatibility.md)。

- 在保持 deterministic MIR 与 checked error order 的前提下扩展 optimizer analysis。
- 强化 target-specific LLVM calling convention 与 data-layout reporting。
- 只以未来明确 versioned ABI addition 的方式评估 WASM checked bounds/status。
- 改进 debug/source mapping 与 artifact introspection。
- 增加 conformance fixture、fuzzing 与可复现性能历史。
- 定义未来 1.0 language/ABI commitment 的准入要求。
- Cross-compilation、program argument、更丰富 I/O 与 public JIT API 仅通过独立 versioned
  design 评估。

任何条目在进入 versioned design 与 release contract 前都没有交付日期或兼容效力。
