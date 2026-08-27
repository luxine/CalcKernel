# CalcKernel Compiler Architecture

[简体中文](../zh-CN/compiler/architecture.md)

This document explains the native Rust compiler organization. Public compiler
behavior is defined by the reference and ABI documents, not by module placement.

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker
    -> MIR lowering/validation -> MIR optimizer
    +-> C source/header
    +-> WAT/WASM
    +-> structural LLVM -> TargetMachine object -> ORC or in-process LLD
```

`src/frontend/` owns source coordinates, stable diagnostics, tokenization, AST,
parsing, symbol scopes, type checking, definite-return/unreachable analysis,
and typed metadata. Parsing accepts syntactic type forms; type checking resolves
all names and enforces strict value/control-flow rules.

`src/ir/` owns MIR model, lowering, deterministic printing, and validation.
Structured `if`, `while`, `break`, and `continue` become branches and
`MirTerminator::Jump`. Void calls/returns remain targetless/valueless.
The source return-only type is `void`.
`MirType::Slice`, `MakeSlice`, `SliceIndex`, and `Subslice` carry typed descriptor
operations without adding backend-specific checks.

`src/optimizer/` owns the optimization context, analyses, pass definitions,
pipeline selection, per-pass validation, and debug records. A pass may improve
MIR but cannot change diagnostic, mode, evaluation-order, error-order, or ABI
semantics.

`src/backend/` owns C, WASM, Native ABI classification, and LLVM lowering. Each
backend consumes validated MIR and makes its representation explicit. C and
Native implement checked status modes; WASM is unchecked-only. Native LLVM is
structurally built, verified before and after optimization, and emitted to
object bytes by the host TargetMachine.

`src/backend/llvm/` and `native/bridge/` provide typed ownership across the
Rust/C++ boundary. `native/runtime/` owns entry, status diagnostics, and print
effects; LLD and ORC are used in process. `src/cli/` owns argument parsing,
dispatch, transactional output, isolated run/cache policy, and stdout/stderr.
`src/bin/ckc.rs` is a thin process entry and `src/lib.rs` exposes intentional
Rust re-exports.

Diagnostics flow forward without later rewriting. Filesystem, unsupported-mode,
backend, link, and runtime failures are CLI errors. All CK-visible generated
memory is caller-owned; the minimal runtime supplies no allocator.
