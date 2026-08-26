# CK / CalcKernel WASM ABI

[Simplified Chinese](zh-CN/WASM_ABI.md)

This document defines the WASM ABI emitted by `native ckc`.

## Generation

```sh
ckc emit-wat examples/wasm_scalar.ck --out build/scalar.wat
ckc emit-wasm examples/wasm_scalar.ck --out build/scalar.wasm
```

The Rust backend emits WAT and assembles WASM through the Rust `wat` crate.
No external `wat2wasm` executable is required. The resulting `.wasm` file needs
a WebAssembly runtime to run.

## Scope

The current WASM backend supports:

- `i32`, `u32`, `i64`, `u64`, `bool`, `f64`
- `ptr<T>` as a linear-memory byte offset
- exported functions
- non-exported internal functions
- exported linear memory
- deterministic CK struct layout
- unchecked arithmetic
- scalar control flow and function calls
- `ptr<T>` load/store, index access, and struct field access

It does not provide:

- checked overflow lowering
- WASI imports
- heap allocation
- strings
- bounds checks
- `slice<T>`
- a runtime library
- SIMD, threads, GC, or exceptions

## Type Mapping

| CK type | WASM value type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `bool` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `f64` | `f64` |
| `ptr<T>` | `i32` byte offset |

Signedness is selected by instruction choice for division, remainder, and
comparisons. `bool` uses `0` for false and nonzero for true.

## Function ABI

Exported CK functions are exported from the WASM module with their source names.

```ck
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

The emitted function uses WASM `i64` parameters and result. `f64` CK values use
WASM `f64`; pointer values use `i32` byte offsets.

A CK void function has no `(result ...)` declaration. Its valueless returns
leave no stack value, and a targetless void call emits `call` without a following
`local.set`. This applies to single-block functions, dispatcher fallback, and
the structured-while path.

## Memory ABI

The module exports one linear memory:

```wat
(memory (export "memory") 1)
```

CK uses caller-owned memory. The host is responsible for:

- choosing non-overlapping input and output offsets
- writing input structs and arrays into memory
- passing `ptr<T>` values as byte offsets
- reading output buffers after the call
- growing memory if the chosen layout needs more pages

The compiler does not allocate, validate, grow, or free host buffers.

## Struct Layout

WASM uses deterministic CK layout, independent of the host C compiler.

Primitive layout:

| Type | Size | Alignment |
| --- | ---: | ---: |
| `i32` | 4 | 4 |
| `u32` | 4 | 4 |
| `bool` | 4 | 4 |
| `ptr<T>` | 4 | 4 |
| `i64` | 8 | 8 |
| `u64` | 8 | 8 |
| `f64` | 8 | 8 |

Struct fields are laid out in declaration order. Each field offset is aligned to
that field's alignment, and the final size is padded to the struct alignment.

## Host Interop

Any host with a WebAssembly runtime can instantiate the module. Use typed-array
views for homogeneous buffers and `DataView` for mixed-width structs or
byte-exact checks. WebAssembly memory is little-endian.

For `ptr<f64>` buffers:

- the pointer argument is an `i32` byte offset
- `f64` size is 8 bytes
- `ptr<f64>[i]` addresses `base + i * 8`
- a `Float64Array` index is `byteOffset / 8`
- recreate typed-array views after `memory.grow`

## Checked Overflow

The WASM backend is unchecked-only. If checked arithmetic is required, use the C
backend:

```sh
ckc emit-c input.ck --overflow checked --out build/input.c --header build/input.h
ckc build input.ck --overflow checked --out build/input
```

## Slice ABI

A logical `slice<T>` parameter expands to two collision-safe `i32` parameters:
the linear-memory data address followed by its `u32` length. Locals, temporaries,
calls, and dispatcher returns carry the same pair. Internal slice returns use
WASM multi-value `(result i32 i32)` in data/length order.

A stored descriptor occupies 8 bytes with alignment 4: address at offset 0 and
length at offset 4. Slice indexing and sub-slicing use the deterministic element
size. Memory remains caller-owned and descriptors may alias. WASM supports only
`--bounds unchecked`; `--bounds checked` is rejected before emission and no
implicit guard or trap is added.
