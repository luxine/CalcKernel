# 选择 CalcKernel Backend

[English](../../guides/backend-selection.md)

C 最适合广泛 native FFI，也是在需要 checked integer arithmetic 或 checked slice
bounds 时的选择。`emit-c` 生成可检查 C/header；`build` 通过 `clang` 构建 dynamic
library。

WASM 适合在 WebAssembly runtime 中 sandboxed portable execution。`emit-wat`
可读，`emit-wasm` 可直接 instantiate；host 管理 linear memory 并传 byte address。
WASM 只支持 unchecked。

LLVM 适合 textual IR、native object 集成或从 IR 构建 dynamic library。
`emit-llvm` 不需要 `clang`，`build-llvm` 需要；target triple 与 consumer toolchain
必须匹配。LLVM 只支持 unchecked。

| 需求 | 推荐命令 |
| --- | --- |
| Syntax/type validation | `ckc check input.ck` |
| Compiler/debug inspection | `ckc emit-mir input.ck -O0..O3` |
| Portable native source/FFI | `ckc emit-c input.ck --out input.c` |
| Checked overflow/bounds | C 配合对应 checked flag |
| WASM runtime | `ckc emit-wasm input.ck --out input.wasm` |
| Native object | `ckc build-llvm input.ck --kind object --out input.o` |

所有输出保持 source evaluation order 与严格 typing，但 ABI shape 不同；请查阅
[ABI 索引](../index.md#abi)。
