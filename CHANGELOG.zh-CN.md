# 变更日志

这里记录 CalcKernel 面向用户的重要变更。

## 0.9.0 - 2026-08-26

- 新增 `while` 循环内的结构化控制语句 `break` 与 `continue`。
- 新增显式 `void` 过程、空 `return;` 以及过程调用语句。
- 新增非 owning 的 `slice<T>` 值、`slice(data, len)` 构造、索引、`.data` / `.len`
  访问，以及写作 `items[start..end]` 的半开 sub-slice。
- C backend 新增可选的 `--bounds checked` slice 边界检查；unchecked 仍是默认值，
  WASM 与 LLVM 会拒绝 checked bounds。
- 冻结原生 C、WebAssembly 与 LLVM 输出路径及其 V0.9 ABI。
- 按稳定的 compiler、contract、example、benchmark 与 test 职责整理仓库，同时保持
  compiler 的公共行为不变。
- 冻结 V0.9 兼容边界：`0.9.x` patch release 保持已接受源码、diagnostic ID、CLI
  行为、文本 MIR 和已记录 ABI contract 的向后兼容。后续 `0.10.0` 只可在提供迁移
  指南时引入已记录的破坏性变更；长期兼容承诺从未来的 `1.0.0` 开始。
- 为 macOS、Linux、Windows 的 arm64 与 x64 提供经过验收的原生 `ckc` 发布归档及
  SHA-256 checksum。
