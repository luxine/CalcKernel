# CalcKernel 0.10 Compatibility Policy

[简体中文](../zh-CN/project/compatibility.md)

This document is the normative compatibility authority for `0.10.x`.

Patch releases preserve accepted 0.10.0 source and observable semantics, stable
diagnostic identifiers and categories, documented CLI names/flags/defaults,
stdout/stderr classes and success/failure behavior, textual MIR, public C/WASM/
Native C ABI shapes, checked first-error order, runtime diagnostic bytes and
exit statuses, and the six release archive names plus checksum sidecars.

Patch releases may reject previously accepted invalid input, improve diagnostic
prose or caret precision, add opt-in commands and APIs, fix code generation, and
optimize when default behavior and every promised semantic boundary remain
unchanged. Private Rust modules, algorithms, tests, cache contents/eviction,
benchmark measurements, and undocumented compiler IR are not public contracts.

## 0.9.0 to 0.10 migration

The intentional changes are:

- `build` no longer invokes Clang; it uses pinned LLVM/LLD in process and still
  defaults to a dynamic library.
- `--kind executable|dynamic|static|object` adds Native artifacts; library forms
  use one generated-header Native C ABI.
- `run`, parameterless internal `main`, and seven Native print builtins are new.
  `main`, `print_i32`, `print_i64`, `print_u32`, `print_u64`, `print_f64`,
  `print_bool`, and `print_newline` are now reserved; conflicting declarations
  must be renamed.
- `build-llvm` remains as a deprecated dynamic/object alias and emits one
  warning; it is not a separate backend.
- Native accepts checked overflow/bounds and uses the existing C status meanings.
- The former standalone textual LLVM exported shape is retired. Native object,
  static, and dynamic libraries use the Native C ABI; `emit-llvm` is inspection
  output and host-only.
- Native build no longer leaves `.c`/`.ll` intermediates. `emit-c` remains
  source-only, and `emit-llvm` remains the explicit IR inspection command.
- C and WebAssembly gain no runtime printing; reachable print from their artifact
  roots is rejected.

An unaffected 0.9.0 source program retains its source semantics. Programs that
used a newly reserved name or depended on the old LLVM export shape require the
migration above. Compatibility fixtures under `tests/fixtures/compatibility`
cover every intentional change.

A future `1.0.0` begins the long-term stability commitment. The 0.10 line does
not claim 1.0 compatibility.
