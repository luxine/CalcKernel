# CalcKernel 0.14.0 文档

[English](../index.md)

这些文档描述当前 0.14.0 产品契约。设计和实施历史由 Git history 保留，不在 release tree 中
保留 planning document。

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
- [WASM interop](guides/wasm-interop.md)
- [Performance](guides/performance.md)

## Project

- [兼容性](project/compatibility.md) — `0.14.x` 规范性权威及保留的 0.13/0.12/0.11/0.10 migration boundary。
- [Release](project/release.md) 与 [checklist](project/release-checklist.md)
- [约定](project/conventions.md)
- [Roadmap](project/roadmap.md) — 非规范的未来可能性。
