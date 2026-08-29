# CalcKernel Roadmap

[简体中文](../zh-CN/project/roadmap.md)

This document is non-normative and lists only undelivered possibilities. It does
not override the [0.11 compatibility policy](compatibility.md).

- Add source SIMD types, target-specific superword/vector cost models, PGO, and
  auto-tuning only through separately reviewed future contracts.
- Harden target-specific LLVM calling conventions and data-layout reporting.
- Evaluate checked bounds/status support for WASM only as a future, explicitly
  versioned ABI addition.
- Improve debug/source mapping and artifact introspection.
- Add more conformance fixtures, fuzzing, and reproducible performance history.
- Define the requirements for a future 1.0 language and ABI commitment.
- Evaluate cross-compilation, program arguments, richer I/O, and a public JIT
  API only through separately versioned designs.

Items have no delivery date or compatibility effect until accepted into a
versioned design and released contract.
