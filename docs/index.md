# CalcKernel 0.12.0 Documentation

[简体中文](zh-CN/index.md)

These documents describe the current 0.12.0 product contract. Git history,
rather than release-tree planning documents, records design and implementation
history.

## Language and commands

- [Language](reference/language.md) — source types, control flow, entry, print, and memory boundary.
- [Diagnostics](reference/diagnostics.md) — stable frontend diagnostic identifiers.
- [CLI](reference/cli.md) — commands, defaults, artifacts, cache, and failures.
- [MIR and KIR](reference/mir.md) — stable semantic MIR boundary and internal verified KIR.

## ABI

- [Native LLVM and C ABI](abi/llvm.md) — host-native lowering, public thunks, artifacts, and ORC.
- [C source ABI](abi/c.md) — source-only generated C and header.
- [WebAssembly ABI](abi/wasm.md) — module and caller-owned linear memory.
- [Checked modes](abi/modes.md) — C/Native status, ordering, and runtime mapping.

## Compiler and guides

- [Architecture](compiler/architecture.md) — compiler ownership and data flow.
- [Optimizer](compiler/optimizer.md) — O0–O3 selection and preservation rules.
- [Getting started](guides/getting-started.md)
- [Backend selection](guides/backend-selection.md)
- [WASM interop](guides/wasm-interop.md)
- [Performance](guides/performance.md)

## Project

- [Compatibility](project/compatibility.md) — normative `0.12.x` authority and
  retained 0.11/0.10 migration boundaries.
- [Release](project/release.md) and [checklist](project/release-checklist.md)
- [Conventions](project/conventions.md)
- [Roadmap](project/roadmap.md) — non-normative future possibilities.
