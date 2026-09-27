# CalcKernel 0.14 WebAssembly ABI

[简体中文](../zh-CN/abi/wasm.md)

This document defines WAT/WASM emitted by `emit-wat` and `emit-wasm`. A module
exports source `export fn` functions and one caller-owned linear memory;
internal functions remain internal.

WAT and WASM lower the same verified KIR used by C and Native. The consumer is
selected before KIR construction, so unsupported checked modes and reachable
Native print are rejected before optimization; the backend adds no hidden
guards and has no legacy optimized-MIR path.

## Values and memory

`i32`, `u32`, `bool`, and `ptr<T>` use `i32`; `i64` and `u64` use `i64`; `f64`
uses `f64`; void has no `(result ...)`. A void call is targetless. Signedness selects operations. Pointers are byte
addresses in little-endian linear memory.

The caller owns allocation, validity, alignment, lifetime, growth, and aliases.
It must recreate host views after `memory.grow`. CK supplies no allocator.

## `slice<T>`

A `slice<T>` parameter is two collision-safe `i32` values in data,length order.
Internal calls and multi-value returns preserve the same order. A stored
descriptor is 8 bytes aligned to 4, with address at offset 0 and `u32` length at
offset 4. Address arithmetic uses the deterministic CK/WASM element layout.

WASM accepts `--overflow unchecked` and `--bounds unchecked`; either checked
selection is rejected before output. No implicit slice guard or trap is added.
The C/Native checked status ABI is not part of this ABI.

WebAssembly has no 0.14 runtime printing. A reachable print from an exported
root is rejected. An internal `main` does not create a WASI or browser entry.
