# CalcKernel Compiler Architecture

[简体中文](../zh-CN/compiler/architecture.md)

This document explains the native Rust compiler organization. Public compiler
behavior is defined by the reference and ABI documents, not by module placement.

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker
    -> MIR lowering/validation -> MIR optimizer
    -> C | WAT/WASM | LLVM
    -> optional clang build
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

`src/backend/` owns C, WASM, and LLVM planning/emission. Each backend consumes
validated MIR and makes its ABI representation explicit. C alone implements the
checked status modes. Backend output must be deterministic for identical source,
flags, target, and compiler version.

`src/cli/` owns argument parsing, command dispatch, atomic output, toolchain
invocation, and user-facing stdout/stderr. `src/bin/ckc.rs` is a thin process
entry point. `src/lib.rs` preserves the public Rust re-exports used by tests and
embedders.

Diagnostics flow forward without being rewritten by later stages. File-system,
unsupported-mode, backend, and `clang` failures are CLI errors. All generated
memory is caller-owned; the compiler supplies no runtime allocator.
