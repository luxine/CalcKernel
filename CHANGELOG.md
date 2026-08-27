# Changelog

All notable user-visible changes to CalcKernel are recorded here.

## 0.10.0 - 2026-08-27

- Added parameterless internal `main`, `ckc run`, and Native executable output.
- Added deterministic Native output builtins for signed/unsigned integers,
  `f64`, booleans, and newline; library, C, and WASM roots reject reachable print.
- Replaced product Clang subprocesses with pinned LLVM 22.1.8 structural code
  generation, ORC execution, archive writing, and in-process LLD linking.
- Expanded `ckc build --kind` to executable, dynamic, static, and object outputs;
  dynamic remains the default and `build-llvm` is a deprecated compatibility alias.
- Unified Native object/static/dynamic exports under one generated-header Native
  C ABI with target ABI classification and checked status thunks.
- Added Native checked overflow and slice bounds, preserving the C `CK_Status`
  meanings and first-error order.
- Added an isolated `run` child, secure persistent object cache, fixed runtime
  diagnostics/statuses, eager symbol resolution, and audited JIT page permissions.
- Added checked/unchecked C-oracle performance gates and six-host functional,
  artifact, dependency, provenance, and immutable release gates.
- Reserved `main` and the seven print builtin names, limited Native target output
  to the host, retired the standalone LLVM exported-shape promise, and kept
  `emit-c` source-only. See the compatibility policy for migration guidance.

## 0.9.0 - 2026-08-26

- Added `break` and `continue` for structured control inside `while` loops.
- Added explicit `void` procedures, empty `return;`, and procedure-call statements.
- Added non-owning `slice<T>` values, `slice(data, len)`, indexing, `.data` / `.len`,
  and half-open sub-slices written as `items[start..end]`.
- Added optional checked slice bounds to the C backend through `--bounds checked`;
  unchecked bounds remain the default, while WASM and LLVM reject checked bounds.
- Stabilized native C, WebAssembly, and LLVM output paths and their V0.9 ABIs.
- Reorganized the repository around durable compiler, contract, example, benchmark,
  and test responsibilities without changing the compiler's public behavior.
- Froze the V0.9 compatibility boundary: patch releases in the `0.9.x` line preserve
  accepted source, diagnostic identifiers, CLI behavior, textual MIR, and documented
  ABI contracts. A later `0.10.0` may make documented breaking changes with migration
  guidance; long-term compatibility begins with a future `1.0.0` release.
- Added signed-off native `ckc` release archives and SHA-256 checksums for macOS,
  Linux, and Windows on both arm64 and x64.
