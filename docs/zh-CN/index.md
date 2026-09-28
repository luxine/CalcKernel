# CalcKernel 文档

[English](../index.md)

这些页面描述稳定版 0.15.1 的产品契约。`0.15.x` 兼容策略具有规范性效力；适用的历史
版本契约与迁移边界仍会保留在文档中。

## 语言与命令

- [语言](reference/language.md) — source type、control flow、entry、print 与 memory boundary。
- [Diagnostic](reference/diagnostics.md) — stable frontend diagnostic identifier。
- [CLI](reference/cli.md) — command、default、artifact、cache 与 failure。
- [MIR 与 KIR](reference/mir.md) — stable semantic MIR boundary 与 internal verified KIR。

## ABI

- [Native LLVM 与 C ABI](abi/llvm.md) — host-native lowering、public thunk、artifact 与 ORC。
- [C source ABI](abi/c.md) — source-only generated C/header。
- [WebAssembly ABI](abi/wasm.md) — module 与 caller-owned linear memory。
- [Checked mode](abi/modes.md) — C/Native status、order 与 runtime mapping。

## Compiler 与 guide

- [Architecture](compiler/architecture.md)
- [Optimizer](compiler/optimizer.md)
- [快速开始](guides/getting-started.md)
- [Backend 选择](guides/backend-selection.md)
- [Visual Studio Code](guides/vscode.md) — 安装与配置 CK 语言扩展。
- [WASM interop](guides/wasm-interop.md)
- [Performance](guides/performance.md)

## Project

- [兼容性](project/compatibility.md) — `0.15.x` 规范性权威及保留的 0.14.0/0.13.0/0.12.0/0.11.0/0.10.0 migration boundary。
- [Release](project/release.md) 与 [checklist](project/release-checklist.md)
- [约定](project/conventions.md)
- [Roadmap](project/roadmap.md) — 非规范的未来可能性。
