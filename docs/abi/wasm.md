# CalcKernel V0.9 WebAssembly ABI

[简体中文](../zh-CN/abi/wasm.md)

This document normatively defines WAT/WASM emitted by `emit-wat` and
`emit-wasm`. The module exports CK `export fn` functions and one caller-owned
linear memory. Internal functions remain internal.

## Values and functions

| CK | WebAssembly |
| --- | --- |
| `i32`, `u32`, `bool`, `ptr<T>` | `i32` |
| `i64`, `u64` | `i64` |
| `f64` | `f64` |
| `void` return | no `(result ...)` |

Signedness selects signed or unsigned comparison/division instructions; it is
not a distinct WASM storage type. Pointer values are `i32` byte addresses in
linear memory. A void call is targetless and emits no result.

## Deterministic memory layout

Primitive sizes/alignments are 4/4 for `i32`, `u32`, `bool`, and pointers, and
8/8 for `i64`, `u64`, and `f64`. Struct fields appear in declaration order,
each at its natural aligned offset; total size is rounded to the maximum field
alignment. Memory is little-endian.

The host owns all memory. It chooses byte offsets, writes inputs, calls exports,
and reads results. It must recreate JavaScript typed-array or `DataView` views
after `memory.grow`. CK provides no allocator or lifetime management.

## `slice<T>`

A `slice<T>` parameter becomes two collision-safe `i32` parameters in data,
length order. The length carries source `u32` semantics. Locals, temporaries,
calls, and dispatcher paths preserve the same pair. Internal slice returns use
WASM multi-value `(result i32 i32)` in data,length order.

A stored descriptor is 8 bytes with alignment 4: address at offset 0 and length
at offset 4. Index and sub-slice address arithmetic uses the deterministic
element size. Descriptors are non-owning and may alias.

WASM accepts only `--overflow unchecked` and `--bounds unchecked`; it explicitly
rejects either checked selection before emission. It does not add implicit slice
guards or traps. Checked C behavior is not part of the WASM ABI.
