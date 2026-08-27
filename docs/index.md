# CalcKernel 0.9 Documentation

[简体中文](zh-CN/index.md)

This index separates normative V0.9 contracts from explanatory material.

## Language Reference

- [Language](reference/language.md) — normative source-language contract.
- [Diagnostics](reference/diagnostics.md) — normative stable diagnostic identifiers.
- [CLI](reference/cli.md) — normative command, flag, output, and exit behavior.
- [MIR](reference/mir.md) — normative textual MIR and validation boundary.

## ABI

- [C ABI](abi/c.md) — normative C layout and function contract.
- [WebAssembly ABI](abi/wasm.md) — normative module and linear-memory contract.
- [LLVM ABI](abi/llvm.md) — normative LLVM IR and exported-shape contract.
- [Checked modes](abi/modes.md) — normative overflow, bounds, and status contract.

## Compiler

- [Architecture](compiler/architecture.md) — explanatory compiler organization.
- [Optimizer](compiler/optimizer.md) — normative optimization levels with explanatory implementation notes.
- [0.10 native toolchain design](compiler/native-toolchain-design.md) — approved forward design; not the current V0.9 implementation contract.

## Guides

- [Getting started](guides/getting-started.md) — explanatory build and first-use guide.
- [Backend selection](guides/backend-selection.md) — explanatory integration guide.
- [WASM interop](guides/wasm-interop.md) — explanatory host-memory guide.
- [Performance](guides/performance.md) — explanatory benchmark guide.

## Project

- [Compatibility](project/compatibility.md) — normative authority for the `0.9.x` line.
- [Roadmap](project/roadmap.md) — non-normative, forward-looking work only.
- [Release](project/release.md) — normative native artifact policy.
- [Release checklist](project/release-checklist.md) — normative release sign-off gate.
- [Conventions](project/conventions.md) — normative repository naming and layout rules.
