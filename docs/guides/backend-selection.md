# Choosing a CalcKernel Backend

[简体中文](../zh-CN/guides/backend-selection.md)

Use Native for zero-toolchain execution and FFI. `run` executes `main`; `build`
creates executable, dynamic, static, or object output and supports checked
integer arithmetic and slice bounds. Library artifacts include a C ABI header.

Use C when portable, inspectable source is the integration boundary. `emit-c`
produces C/header files only; the consumer deliberately chooses its own compiler.

Use WASM for sandboxed, portable execution in a WebAssembly runtime. `emit-wat`
is readable; `emit-wasm` is directly instantiable. The host manages linear
memory and passes byte addresses. WASM is unchecked-only.

Use `emit-llvm` only to inspect host-native IR. `build-llvm` is a deprecated
alias for Native dynamic/object output and should not be used by new scripts.

| Need | Recommended command |
| --- | --- |
| Syntax/type validation | `ckc check input.ck` |
| Semantic compiler inspection | `ckc emit-mir input.ck` |
| Optimizer/fact inspection | `ckc emit-kir input.ck -O0..O3 --print-facts` |
| Portable native source/FFI | `ckc emit-c input.ck --out input.c` |
| Run a CK program | `ckc run input.ck` |
| Standalone executable | `ckc build input.ck --kind executable --out app` |
| Checked overflow or slice bounds | Native or C with the corresponding flag |
| Browser/server WASM runtime | `ckc emit-wasm input.ck --out input.wasm` |
| Native library/object | `ckc build input.ck --kind dynamic|static|object --out output` |

All outputs preserve source evaluation order and strict typing. Backend ABI
shapes differ; consult the relevant document under [ABI](../index.md#abi).
