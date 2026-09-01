# CalcKernel 0.12 Compatibility Policy

[简体中文](../zh-CN/project/compatibility.md)

This document is the normative compatibility authority for `0.12.x`.

Patch releases preserve accepted 0.12.0 source and observable semantics, stable
diagnostic identifiers/categories, documented CLI names/flags/defaults,
stdout/stderr classes, semantic textual MIR, public C/WASM/Native C ABI shapes,
checked first-error order, runtime diagnostic bytes/statuses, and the six
release archive names plus checksum sidecars.

Patch releases may reject invalid input, improve diagnostic prose, add opt-in
commands, fix code generation, and optimize when every promised semantic
boundary remains unchanged. Private Rust modules, KIR text, facts/proof encoding,
pass algorithms, private LLVM bridge ABI, cache entries, measurements, and
undocumented compiler interfaces are not public contracts.

## 0.11.0 to 0.12.0 migration

- Accepted 0.11 source, language semantics, semantic MIR, diagnostics, and the
  public Native C ABI remain compatible. Native C ABI stays version 1 and the
  contract-aware runtime ABI stays version 2.
- KIR advances from v1 to v2. Each module now binds a canonical
  `KirTargetProfile`; fixed-vector KIR, specialization, unroll, SLP, Loop SIMD,
  independent checkers, and transactional optimizer audit state are private
  compiler facilities, not a new source vector language or public KIR ABI.
- `emit-kir --consumer inspection|c|wasm|native-library|native-executable`
  selects the exact inspection profile. Native consumers also accept
  `--cpu baseline|native`; the default inspection profile remains scalar and
  target-independent.
- C and WebAssembly remain scalar in 0.12. Native may automatically emit
  fixed-width SIMD only when legality, strict semantics, cost, proof, and budget
  checks close. Checked/sanitizer behavior and observable fallback semantics do
  not change.
- The private LLVM bridge ABI advances from 2 to 3. Native object/run cache
  entries advance to `CKCOBJ02` manifest schema 3 and include target-profile,
  proof/cost schema, and optimizer-budget identity. 0.11 cache entries and old
  bridge clients fail closed; this does not change foreign-call signatures.
- PGO remains 0.13. Auto-Tuning remains 0.14; neither is claimed by 0.12.

Every intentional 0.12 addition maps to executable evidence in
`tests/fixtures/compatibility/v0_12/manifest.toml`. Accepted 0.11 fixtures remain
compiled at the frozen boundary.

## 0.10.0 to 0.11.0 migration

- `unsafe fn` contracts, explicit `unsafe { ... }` calls, and diagnostics
  `CK2014`–`CK2016` were added. Existing safe 0.10 source remained safe source
  and gained no optimizer-assumed undefined behavior.
- `emit-kir`, `--print-facts`, `--print-effect-summaries`, and
  `--explain-optimization` added deterministic inspection. KIR v1 was private.
- `--sanitize-contracts` added opt-in Native run/executable checking with
  `CKR0007`/status 246. Ordinary compilation continued to trust each unsafe
  precondition without inserting a check.
- C, WebAssembly, and Native began consuming one verified fact-driven KIR
  optimizer. Stable semantic `emit-mir` remained compatible.
- Native C ABI stayed version 1; the private LLVM bridge and contract-aware
  runtime ABI advanced to version 2. Exported unsafe functions retained their C
  ABI while generated headers gained normalized contract comments.

The executable history is retained in
`tests/fixtures/compatibility/v0_11/manifest.toml`.

## 0.9.0 to 0.10.0 migration

- Native `build` moved from external Clang to pinned in-process LLVM/LLD and
  added executable, dynamic, static, and object kinds under one Native C ABI.
- `run`, parameterless internal `main`, and seven Native print builtins were
  added; their names became reserved.
- `build-llvm` became a deprecated compatibility alias, checked Native modes
  were added, and the standalone textual LLVM export-shape promise was retired.
- Native stopped leaving `.c`/`.ll` intermediates; `emit-c` stayed source-only;
  C and WebAssembly continued to reject reachable Native print.

A future `1.0.0` begins the long-term stability commitment. The 0.12 line does
not claim 1.0 compatibility.
