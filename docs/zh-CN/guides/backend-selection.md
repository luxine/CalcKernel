# 选择 CalcKernel Backend

[English](../../guides/backend-selection.md)

Native 用于零工具链 execution 与 FFI。`run` 执行 `main`；`build` 生成 executable、
dynamic、static 或 object，并支持 checked integer arithmetic/slice bounds；library artifact
附带 C ABI header。

C 用于 portable、可检查 source integration boundary。`emit-c` 只生成 C/header，consumer
自行选择 compiler。

WASM 适合在 WebAssembly runtime 中 sandboxed portable execution。`emit-wat`
可读，`emit-wasm` 可直接 instantiate；host 管理 linear memory 并传 byte address。
WASM 只支持 unchecked。

`emit-llvm` 只用于检查 host-native IR。`build-llvm` 是 Native dynamic/object output 的
deprecated alias，新脚本不应使用。

| 需求 | 推荐命令 |
| --- | --- |
| Syntax/type validation | `ckc check input.ck` |
| Compiler/debug inspection | `ckc emit-mir input.ck -O0..O3` |
| Portable native source/FFI | `ckc emit-c input.ck --out input.c` |
| 运行 CK program | `ckc run input.ck` |
| Standalone executable | `ckc build input.ck --kind executable --out app` |
| Checked overflow/bounds | Native 或 C 配合对应 checked flag |
| WASM runtime | `ckc emit-wasm input.ck --out input.wasm` |
| Native library/object | `ckc build input.ck --kind dynamic|static|object --out output` |

所有输出保持 source evaluation order 与严格 typing，但 ABI shape 不同；请查阅
[ABI 索引](../index.md#abi)。
