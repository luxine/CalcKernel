# CalcKernel Roadmap

[简体中文](../zh-CN/project/roadmap.md)

This document is non-normative and lists only undelivered possibilities. It does
not override the [0.14 compatibility policy](compatibility.md).

- Evaluate indirect calls and indirect-call promotion without weakening the
  closed effect, ABI, or profile-mapping contracts.
- Evaluate scalable KIR vectors separately from 0.14 fixed-width target variants.
- Evaluate adaptive JIT PGO separately; 0.14 profiles and tuning decisions are offline and immutable.
- Add source SIMD types and richer target-specific vector facilities only
  through separately reviewed future contracts.
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
