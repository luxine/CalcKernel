# CalcKernel V0.9 WebAssembly ABI

[English](../../abi/wasm.md)

本文档规范 `emit-wat` / `emit-wasm` 的输出。Module 导出 CK `export fn` 与一个
caller-owned linear memory；internal function 不导出。

`i32`、`u32`、`bool`、`ptr<T>` 映射为 WASM `i32`；`i64`、`u64` 映射为
`i64`；`f64` 映射为 `f64`；void function 没有 `(result ...)`，call 为 targetless。
Signedness 通过 comparison/division instruction 表达。Pointer 是 linear memory
中的 `i32` byte address。

Primitive size/alignment：`i32`、`u32`、`bool`、pointer 为 4/4；`i64`、`u64`、
`f64` 为 8/8。Struct field 按 declaration order 以 natural alignment 排列，总
size 向最大 field alignment 对齐。Memory 是 little-endian。

Host 选择 offset、写 input、调用 export 并读 output；CK 不提供 allocator 或
lifetime management。`memory.grow` 后必须重建 typed-array/`DataView` view。

`slice<T>` parameter flatten 为 collision-safe 的两个 `i32`，顺序为 data、length；
length 保持 source `u32` semantics。Local、temporary、call 与 dispatcher path
保持相同 pair。Internal slice return 使用 WASM multi-value
`(result i32 i32)`，顺序仍为 data、length。Stored descriptor 大小 8、alignment 4，
offset 0 为 address，offset 4 为 length。Descriptor 非 owning 且可 alias。

WASM 只接受 `--overflow unchecked` 与 `--bounds unchecked`，会在 emission 前
明确 reject checked selection；不会插入 implicit slice guard 或 trap。
