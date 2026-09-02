# CalcKernel 0.14 C Source ABI

[简体中文](../zh-CN/abi/c.md)

This document defines the C and header produced by `ckc emit-c`. This path is
source-only: it never compiles or links. Native `ckc build` instead emits LLVM
objects directly and follows the [Native C ABI](llvm.md).

The C backend consumes verified KIR. It does not reconstruct hidden checks or
run a separate MIR optimizer. Verified complete pairwise `noalias` and alignment
facts may appear as portable `restrict`/alignment hints; incomplete or stale
facts emit no hint. Exported unsafe functions keep the same ABI and their header
declarations include normalized contract comments.

## Mapping

CK `i32`, `i64`, `u32`, `u64`, `f64`, and `bool` map to the corresponding
fixed-width C integers, `double`, and `bool`. `ptr<T>` maps to `T*`; structs
become declaration-order typedef structs; a void return maps to C `void` in
unchecked mode. The generated header supplies `CK_API` visibility and C++
`extern "C"` guards.

The host compiles and includes the header rather than guessing target padding.
Memory reachable through pointers and `slice<T>` is caller-owned and may alias.
Stored slice descriptors contain `T* data` then `uint32_t len`; slice parameters
are flattened in data,length order. Exported slice returns are invalid.

## Calls and checked status

Unchecked exports return their source result directly. Whenever overflow or
bounds checking is selected, the full generated module uses `CK_Status`: a
non-void function appends a result pointer named `ck_return` and a void function does not. Calls
propagate the first non-OK status. Exact codes and order are defined by
[Checked modes](modes.md).

`--bounds checked` covers slice indexing and half-open sub-slices only. Raw pointer
indexing, `slice(data, len)`, indexing through `.data`, memory validity,
alignment, lifetime, and declared length remain caller responsibilities.

The C backend has no runtime output implementation. A reachable print from any
C artifact root is rejected before output. A valid internal `main` may be
lowered as an ordinary function, but `emit-c` creates no process entry.
