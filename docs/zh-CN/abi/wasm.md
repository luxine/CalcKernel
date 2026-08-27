# CalcKernel 0.10 WebAssembly ABI

[English](../../abi/wasm.md)

本文档定义 `emit-wat` 与 `emit-wasm` 输出。Module export 源码中的 `export fn`，并提供一块
caller-owned linear memory；internal function 保持 internal。

`i32`、`u32`、`bool`、`ptr<T>` 使用 WASM `i32`；`i64`/`u64` 使用 `i64`；`f64`
使用 `f64`；void 无 `(result ...)`，void call 为 targetless。Pointer 是 little-endian linear memory 中的 byte address。
Caller 负责 allocation、validity、alignment、lifetime、growth 与 alias，并在 `memory.grow`
后重建 host view；CK 不提供 allocator。

`slice<T>` parameter 是 data,length 顺序的两个 collision-safe `i32`。Stored descriptor 为
8 bytes、alignment 4，offset 0 为 address，offset 4 为 `u32` length。Internal call 与
multi-value return 保持同一顺序。

WASM 只接受 `--overflow unchecked` 与 `--bounds unchecked`；任一 checked selection 在
输出前 rejected，不插入隐式 slice guard 或 trap。C/Native checked status ABI 不属于本 ABI。

WebAssembly 0.10 没有 runtime print；export root 可达的 print 被拒绝。Internal `main`
不会创建 WASI 或 browser entry。
