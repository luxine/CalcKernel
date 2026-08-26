# CalcKernel V0.9 C ABI

[English](../../abi/c.md)

本文档规范 `ckc emit-c` 与 `ckc build` 使用的 generated C/header shape。

| CK | C |
| --- | --- |
| `i32` / `i64` | `int32_t` / `int64_t` |
| `u32` / `u64` | `uint32_t` / `uint64_t` |
| `f64` | `double` |
| `bool` | `bool` |
| `ptr<T>` | `T*` |
| named struct | declaration-order field 的 named `typedef struct` |
| return-only `void` | unchecked mode 中的 C `void` |

Header 使用 `#pragma once`、`<stdint.h>`、`<stdbool.h>`，checked status ABI 还
使用 `<stddef.h>`。Export declaration 使用 `CK_API`：Windows 映射到
`__declspec(dllexport/dllimport)`，ELF/Mach-O 映射到 default visibility，并以
`extern "C"` 支持 C++。`ckc build` 定义 `CK_BUILD_DLL`。Internal function 为
`static`，不进入 header。

Struct field 顺序稳定，byte offset 与总 alignment 遵循 target C compiler 对映射
type 的 ABI。Host 应 include generated header，不应猜测 padding。`i32_to_f64` 与
`u32_to_f64` 精确 lower 为 C `double` cast，不改变函数 shape。

Unchecked exported function 直接返回映射后的 source type；void procedure 为 C
`void`。Overflow 或 bounds 任一为 checked 时，整个 module 使用
[status ABI](modes.md)：非 void function 返回 `CK_Status` 并追加 `T* ck_return`；
void function 返回 `CK_Status` 且没有 `ck_return`，call 传播首个非 `CK_OK` status。

Pointer 与 slice data 非 owning 且可 alias。Caller 负责 allocation、lifetime、
alignment、element type 与有效范围。Stored `slice<T>` descriptor 依次包含
`T* data` 与 `uint32_t len`；每个 exported/internal slice parameter 都物理 flatten
为 `(T* data, uint32_t len)`。Unchecked internal slice return 按 descriptor value
返回；checked internal return 使用最终 descriptor output pointer；exported slice
return 非法。

`--bounds checked` 只保护 slice index 与 sub-slice。Raw pointer indexing、
`slice(data, len)` 与通过 `.data` indexing 不检查。Zero-start sub-slice 保留原始
pointer bit。`ckc build` 通过 `clang` 创建 platform dynamic library；ABI 不提供
allocator、standalone executable 或 host wrapper。
