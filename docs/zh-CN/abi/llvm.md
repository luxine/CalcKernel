# CalcKernel V0.9 LLVM ABI

[English](../../abi/llvm.md)

本文档规范 `emit-llvm` 的 textual LLVM IR 与 `build-llvm` 的 object/library。
IR 使用 opaque pointer 与显式/自动检测的 target triple；最终 platform calling
detail 由该 triple 与 `clang` 决定。

`i32`/`u32` 映射为 LLVM `i32`，`i64`/`u64` 为 `i64`，`f64` 为 `double`，
`bool` 为 `i1`，`ptr<T>` 为 `ptr`，named struct 为 declaration-order 的 named
LLVM struct，`void` 为 LLVM `void`。Signedness 通过 comparison/division/remainder
instruction 选择表达。转换使用 `sitofp` / `uitofp`，不添加 fast-math flag。

Exported source function 使用 external definition，internal function 使用 internal
linkage。Void procedure 生成 `define void`，targetless call 生成 `call void`，显式
或自然结束生成 `ret void`。Exported bool parameter/return 在 V0.9 中是 plain `i1`；
consumer 必须匹配 emitted target 与 IR shape。

Pointer 是 opaque `ptr`，但 GEP/load/store element type 遵循 CK type。Stored
`slice<T>` 使用 `{ ptr, i32 }`。每个 physical slice parameter flatten 为 data、
length 顺序的 `ptr, i32` 并用 `insertvalue` 重建。Move、struct field、memory
operation、call 与 internal aggregate return 都保留 descriptor；exported slice
return 非法。

Index/sub-slice GEP 使用实际 element type；zero-start sub-slice 保留原 pointer
bit。Memory 由 caller 拥有且可 alias。

LLVM 只接受 `--overflow unchecked` 与 `--bounds unchecked`，会在 IR emission 前
明确 reject checked mode；不会添加 slice guard、trap 或 C status ABI。
`build-llvm --kind dynamic|object` 由 `clang` 完成 target validation 与产物构建。
