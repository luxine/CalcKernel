# Choosing a CalcKernel Backend

[简体中文](../zh-CN/guides/backend-selection.md)

Use C for the broadest native FFI and whenever checked integer arithmetic or
checked slice bounds are required. `emit-c` produces inspectable C/header files;
`build` invokes `clang` for a dynamic library.

Use WASM for sandboxed, portable execution in a WebAssembly runtime. `emit-wat`
is readable; `emit-wasm` is directly instantiable. The host manages linear
memory and passes byte addresses. WASM is unchecked-only.

Use LLVM for textual IR, native object integration, or a dynamic library built
from LLVM IR. `emit-llvm` does not require `clang`; `build-llvm` does. Match the
target triple and consumer toolchain. LLVM is unchecked-only.

| Need | Recommended command |
| --- | --- |
| Syntax/type validation | `ckc check input.ck` |
| Compiler/debug inspection | `ckc emit-mir input.ck -O0..O3` |
| Portable native source/FFI | `ckc emit-c input.ck --out input.c` |
| Checked overflow or slice bounds | C with the corresponding checked flag |
| Browser/server WASM runtime | `ckc emit-wasm input.ck --out input.wasm` |
| Native object/link pipeline | `ckc build-llvm input.ck --kind object --out input.o` |

All outputs preserve source evaluation order and strict typing. Backend ABI
shapes differ; consult the relevant document under [ABI](../index.md#abi).
