# CalcKernel 0.11 Compatibility Policy

[简体中文](../zh-CN/project/compatibility.md)

This document is the normative compatibility authority for `0.11.x`.

Patch releases preserve accepted 0.11.0 source and observable semantics, stable
diagnostic identifiers/categories, documented CLI names/flags/defaults,
stdout/stderr classes, semantic textual MIR, public C/WASM/Native C ABI shapes,
checked first-error order, runtime diagnostic bytes/statuses, and the six
release archive names plus checksum sidecars.

Patch releases may reject invalid input, improve diagnostic prose, add opt-in
commands, fix code generation, and optimize when every promised semantic
boundary remains unchanged. Private Rust modules, KIR text, facts/proof encoding,
pass algorithms, private LLVM bridge ABI, cache entries, measurements, and
undocumented compiler interfaces are not public contracts.

## 0.10.0 to 0.11 migration

- `unsafe fn` contracts, explicit `unsafe { ... }` calls, and diagnostics
  `CK2014`–`CK2016` are new. Existing safe 0.10 source remains safe source and
  gains no optimizer-assumed undefined behavior.
- `emit-kir`, `--print-facts`, `--print-effect-summaries`, and
  `--explain-optimization` expose deterministic inspection only. KIR is not a
  stable cross-version interchange format.
- `--sanitize-contracts` is opt-in for Native run/executable debugging and adds
  private runtime diagnostic `CKR0007`/status 246. Ordinary compilation trusts
  each unsafe precondition and inserts no check.
- C, WebAssembly, and Native now consume one verified fact-driven KIR optimizer.
  Stable semantic `emit-mir` remains compatible; the retired optimized-MIR
  product path was never a separate public ABI.
- Native C ABI remains version 1. The private LLVM bridge and contract-aware
  runtime ABI become version 2, and native cache/code-generation identity uses
  KIR v1, so 0.10 cached objects are intentionally not reused.
- Exported unsafe functions retain their ordinary C ABI, while generated headers
  add normalized contract comments. Foreign callers assume the entry obligation.

Every intentional 0.11 addition maps to executable evidence in
`tests/fixtures/compatibility/v0_11/manifest.toml`. Accepted 0.10 fixtures remain
compiled at the frozen boundary.

## 0.9.0 to 0.10 migration

The previous migration remains historical guidance:

- Native `build` moved from external Clang to pinned in-process LLVM/LLD and
  added executable, dynamic, static, and object kinds under one Native C ABI.
- `run`, parameterless internal `main`, and seven Native print builtins were
  added; their names became reserved.
- `build-llvm` became a deprecated compatibility alias, checked Native modes
  were added, and the standalone textual LLVM export-shape promise was retired.
- Native stopped leaving `.c`/`.ll` intermediates; `emit-c` stayed source-only;
  C and WebAssembly continued to reject reachable Native print.

A future `1.0.0` begins the long-term stability commitment. The 0.11 line does
not claim 1.0 compatibility.
