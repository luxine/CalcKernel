# CalcKernel V0.9 C ABI

[简体中文](../zh-CN/abi/c.md)

This document normatively defines generated C and header shapes. It applies to
`ckc emit-c` and the C used by `ckc build`.

## Type and layout mapping

| CK | C |
| --- | --- |
| `i32` | `int32_t` |
| `i64` | `int64_t` |
| `u32` | `uint32_t` |
| `u64` | `uint64_t` |
| `f64` | `double` |
| `bool` | `bool` |
| `ptr<T>` | `T*` |
| named struct | named `typedef struct`, fields in declaration order |
| return-only `void` | C `void` in unchecked mode |

Generated headers use `#pragma once`, `<stdint.h>`, and `<stdbool.h>`; a checked
status ABI also uses `<stddef.h>`. Exported declarations use `CK_API`, mapping to
Windows `__declspec(dllexport/dllimport)` or default ELF/Mach-O visibility, and
are wrapped in `extern "C"` for C++ consumers. `ckc build` defines
`CK_BUILD_DLL`. Internal functions are `static` and absent from the header.

Struct field order is stable, while byte offsets and total alignment follow the
target C compiler's ABI for the mapped field types. Hosts must compile/include
the generated header rather than guess target padding.

Exact `i32_to_f64` and `u32_to_f64` builtins lower to C casts to `double` and do
not alter function shape.

## Function shapes

Unchecked exported functions return their mapped source type directly. A source
void procedure becomes C `void`, uses ordinary calls and `return;`.

Whenever overflow or bounds mode is checked, the entire generated C module uses
the [status ABI](modes.md). A non-void function returns `CK_Status` and appends
`T* ck_return`; a void function returns `CK_Status` without `ck_return`. Calls
propagate the first non-`CK_OK` status.

## Pointers, slices, and ownership

Pointers and slice data are non-owning and may alias. CK never allocates, frees,
or retains caller buffers. The host is responsible for allocation, lifetime,
alignment, correct element type, and valid pointer ranges.

Stored `slice<T>` values use a deterministic generated descriptor containing
`T* data` followed by `uint32_t len`. Every exported and internal slice parameter
is physically flattened to `(T* data, uint32_t len)` and reconstructed in the
body. Unchecked internal slice returns use the descriptor by value. Checked
internal slice returns use a final descriptor output pointer. Exported slice
returns are invalid CK.

`--bounds checked` guards only slice indexing and sub-slicing. Raw pointer
indexing, `slice(data, len)`, and indexing through `slice_value.data` remain
unchecked. A zero-start sub-slice preserves the original data pointer bits,
including an empty descriptor's pointer.

## Dynamic libraries

`ckc build` compiles the generated C and header with `clang` into the platform
dynamic-library form. The caller loads exported `CK_API` symbols and obeys the
exact generated header. The C ABI does not promise a standalone executable,
memory allocator, host wrapper, or per-language binding.
