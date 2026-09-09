# CalcKernel 0.13 Compatibility Policy

[简体中文](../zh-CN/project/compatibility.md)

This document is the normative compatibility authority for `0.13.x`.

Patch releases preserve accepted 0.13.0 source and observable semantics, stable
diagnostic identifiers/categories, documented CLI names/flags/defaults,
stdout/stderr classes, semantic textual MIR, public C/WASM/Native C ABI shapes,
checked first-error order, runtime diagnostic bytes/statuses, and the six
release archive names plus checksum sidecars.

Patch releases may reject invalid input, improve diagnostic prose, add opt-in
commands, fix code generation, and optimize when every promised semantic
boundary remains unchanged. Private Rust modules, KIR text and schema, profile
wire formats, facts/proof encoding, pass algorithms, private LLVM bridge ABI,
cache entries, dispatch/collection runtimes, measurements, and undocumented
compiler interfaces are not public contracts.

## 0.12.0 to 0.13.0 migration

- Accepted 0.12 source, language semantics, semantic MIR, diagnostics, checked
  behavior, runtime output, and public Native C ABI remain compatible. Native C
  ABI stays version 1 and Runtime ABI stays version 2.
- KIR advances from v2 to v3. CK workload profile annotations, site mappings,
  O2 `CkLateProfileLayout`, O3 PGO transactions, multiversion bundles, and
  dispatch plans are private compiler facilities, not source-language promises.
- PGO is explicit through `ckc pgo build|merge|inspect`, `--pgo-generate`, and
  `--pgo-use`; ordinary commands remain profile-free. Profile use accepts O2/O3,
  while specialization and `--cpu multiversion` require O3.
- `--cpu` accepts `baseline|native|multiversion` for Native build/inspection.
  Multiversion output is executable/dynamic/static; a multiversion object is
  rejected. Baseline/native single-version profile-use objects stay supported.
- `CKPART01`/`CKPROF01` schema 1 are compiler-owned workload formats. An old,
  mismatched, corrupt, partial, or unknown profile fails closed and is never
  treated as safety evidence.
- The private LLVM bridge advances from ABI 3 to ABI 4. Native cache advances
  from `CKCOBJ02` key/manifest schema 3 to `CKCOBJ03` key/manifest schema 4 and
  binds the complete named-object bundle. Old cache/bridge/KIR/profile clients
  fail closed without changing foreign-call signatures.
- Generation and dispatch runtimes are compiler-private. The generation flush
  symbol and hidden variant symbols do not extend Native C ABI 1 or Runtime ABI 2.
- Auto-Tuning remains 0.14. Indirect-call promotion, scalable KIR vectors, and
  adaptive JIT PGO also remain outside 0.13.

Executable 0.12 compatibility history remains in
`tests/fixtures/compatibility/v0_12/manifest.toml`; its accepted source is
compiled by the current compatibility target.

## 0.11.0 to 0.12.0 migration

- Accepted 0.11 source, semantic MIR, diagnostics, and public Native C ABI stayed
  compatible. Native C ABI stayed version 1 and Runtime ABI stayed version 2.
- KIR advanced from v1 to v2 and bound `KirTargetProfile`; fixed-vector KIR,
  specialization, unroll, SLP, Loop SIMD, and transactional audit state remained
  private facilities. C and WebAssembly remained scalar.
- Native inspection added consumer and baseline/native CPU selection. The
  private LLVM bridge advanced from ABI 2 to 3 and cache advanced to
  `CKCOBJ02` schema 3; 0.11 private entries failed closed.

The executable history is retained in
`tests/fixtures/compatibility/v0_12/manifest.toml`.

## 0.10.0 to 0.11.0 migration

- `unsafe fn` contracts, explicit `unsafe { ... }` calls, and diagnostics
  `CK2014`–`CK2016` were added without giving existing safe source new undefined
  behavior.
- `emit-kir`, fact/effect inspection, optimization explanations, and opt-in
  `--sanitize-contracts`/`CKR0007` were added. Semantic `emit-mir` stayed stable.
- Native C ABI stayed version 1; private LLVM bridge and Runtime ABI advanced to
  version 2. Exported unsafe functions retained their public C ABI.

The executable history is retained in
`tests/fixtures/compatibility/v0_11/manifest.toml`.

## 0.9.0 to 0.10.0 migration

- Native `build` moved from external Clang to pinned in-process LLVM/LLD and
  added executable, dynamic, static, and object kinds under one Native C ABI.
- `run`, parameterless internal `main`, and seven Native print builtins were
  added; `build-llvm` became a deprecated compatibility alias.
- Checked Native modes were added and the standalone textual LLVM export-shape
  promise was retired. C/WebAssembly continued to reject reachable Native print.

The 0.10.0 identity and fixtures remain in
`tests/fixtures/compatibility/v0_10/manifest.toml`. A future `1.0.0` begins the
long-term stability commitment; the 0.13 line does not claim 1.0 compatibility.
