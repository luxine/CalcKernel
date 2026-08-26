# CalcKernel V0.9 LLVM ABI

[简体中文](../zh-CN/abi/llvm.md)

This document normatively defines textual LLVM IR emitted by `emit-llvm` and the
objects/libraries produced by `build-llvm`. IR uses opaque pointers and an
explicit or detected target triple; final platform calling details are governed
by that triple and `clang`.

## Type and function mapping

| CK | LLVM IR |
| --- | --- |
| `i32`, `u32` | `i32` |
| `i64`, `u64` | `i64` |
| `f64` | `double` |
| `bool` | `i1` |
| `ptr<T>` | `ptr` |
| named struct | named LLVM struct, declaration-order fields |
| `void` return | LLVM `void` |

Signedness chooses instructions for comparisons, division, remainder, and
integer-to-float conversion. `i32_to_f64` and `u32_to_f64` lower to `sitofp`
and `uitofp`. No fast-math flags are added.

Exported source functions have external definitions. Internal functions use
internal linkage. A void procedure emits `define void`, targetless calls emit
`call void`, and explicit or natural return emits `ret void`.

Exported `bool` parameters and returns are plain `i1` in V0.9. Consumers must
match the emitted target and IR shape; cross-target attributes inferred by
unrelated toolchains are not promised.

## Pointers and `slice<T>`

Pointers are opaque `ptr` values, but GEP/load/store element types follow the CK
type. Stored `slice<T>` values use `{ ptr, i32 }`. Every physical slice parameter
is flattened to `ptr, i32` in data,length order and reconstructed with
`insertvalue`. Moves, struct fields, memory operations, calls, and internal
aggregate returns preserve the descriptor. Exported slice returns are invalid.

Index and sub-slice GEPs use the actual element type. A zero-start sub-slice
preserves the original pointer bits. Memory remains caller-owned and may alias.

LLVM accepts only `--overflow unchecked` and `--bounds unchecked`; it explicitly
rejects checked modes before IR emission. It does not add slice guards, traps,
or a C-style status ABI. `build-llvm --kind dynamic|object` delegates target
validation and final object/library construction to `clang`.
