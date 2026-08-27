# CalcKernel 0.9 文档

[English](../index.md)

本索引区分规范性 V0.9 contract 与解释性材料。

## 语言参考

- [语言](reference/language.md) — 规范性的源码语言 contract。
- [诊断](reference/diagnostics.md) — 规范性的稳定 diagnostic ID contract。
- [CLI](reference/cli.md) — 规范性的命令、flag、输出与退出行为。
- [MIR](reference/mir.md) — 规范性的文本 MIR 与 validation 边界。

## ABI

- [C ABI](abi/c.md) — 规范性的 C layout 与函数 contract。
- [WebAssembly ABI](abi/wasm.md) — 规范性的 module 与 linear-memory contract。
- [LLVM ABI](abi/llvm.md) — 规范性的 LLVM IR 与导出 shape contract。
- [Checked mode](abi/modes.md) — 规范性的 overflow、bounds 与 status contract。

## Compiler

- [架构](compiler/architecture.md) — 解释性的 compiler 组织。
- [Optimizer](compiler/optimizer.md) — 规范性的优化等级及解释性实现说明。
- [0.10 原生工具链设计](compiler/native-toolchain-design.md) — 已获准的未来设计，不是当前 V0.9 implementation contract。

## 指南

- [入门](guides/getting-started.md) — 解释性的构建与首次使用指南。
- [Backend 选择](guides/backend-selection.md) — 解释性的集成指南。
- [WASM interop](guides/wasm-interop.md) — 解释性的 host memory 指南。
- [性能](guides/performance.md) — 解释性的 benchmark 指南。

## 项目

- [兼容性](project/compatibility.md) — `0.9.x` 的规范性权威。
- [路线图](project/roadmap.md) — 非规范性的未来工作。
- [发布](project/release.md) — 规范性的原生 artifact policy。
- [发布清单](project/release-checklist.md) — 规范性的发布验收门禁。
- [约定](project/conventions.md) — 规范性的仓库命名与布局规则。
