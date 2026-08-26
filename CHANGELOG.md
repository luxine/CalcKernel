# Changelog

All notable user-visible changes to CalcKernel are recorded here.

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
