# CalcKernel 0.15 WebAssembly ABI

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

## Target features and module metadata

`--wasm-features baseline|simd128` selects the WebAssembly target profile for
`emit-wat`, `emit-wasm`, or `emit-kir --consumer wasm`; the default is
`baseline`. The value is part of the canonical KIR target profile and changes
its SHA-256 digest. `baseline` does not emit SIMD. Both profiles allow Bulk
Memory instructions. At O3, `simd128` can emit
128-bit SIMD for independently verified, contiguous `slice<f64>`, `slice<i32>`,
and `slice<u32>` maps, `i32`/`u32` to `f64` casts, and modular integer sum or
product reductions. Full-width operations use `f64x2` and `i32x4` instructions.
Other unsupported SIMD candidates and O0–O2 remain scalar.

Every emitted WAT and binary module carries the `ck.wasm.target` custom
section. Its UTF-8 payload is deterministic JSON, schema 2, with these keys in
this order:

```json
{"schema":2,"target":"wasm32","features":"baseline","profile_sha256":"<64 lowercase hex>"}
```

The `features` value is exactly `baseline` or `simd128`. The digest identifies
the selected canonical KIR target profile. The v0.15 capability encoding
changes the digest for both Wasm profiles; non-Wasm profile digests are
unchanged. The two profiles can also produce different instructions and module
bytes.

The `baseline` allowlist is WebAssembly core MVP plus `MULTI_VALUE`, which is
required by the slice return ABI, and `BULK_MEMORY`. The `simd128` allowlist
adds `SIMD128` to that set. The backend rejects `RELAXED_SIMD`, threads,
Memory64, and every undeclared proposal; profile selection does not relax
instruction validation. Hosts should read and validate `ck.wasm.target`, then
confirm runtime support for the named profile and its v0.15 capability set
before instantiating a module. A host that uses the standard WebAssembly API
can compile first, inspect `WebAssembly.Module.customSections`, and perform
this check before instantiation.

At O3, eligible direct-pointer `i32`/`u32` copy loops and repeated-byte fills
may use a guarded Bulk Memory fast path. The guard proves the complete ranges
are in current Wasm memory without 32-bit address wrap; copy also requires
disjoint source and destination ranges. If the proof fails, the original
scalar loop runs, preserving its overlap behavior and any writes before a
later trap. O0 keeps these loops scalar.

Full-width SIMD memory operations use contiguous 16-byte loads and stores with
proven natural alignment (8 bytes for `f64`, 4 for `i32`/`u32`). Two-lane
`i32`/`u32` to `f64x2` casts read exactly eight source bytes with
`v128.load64_zero`; the converted result stores 16 bytes. Supported lane
operations include splat, add, subtract, multiply, negate, pure comparison and
selection; `f64x2` also supports divide. Eligible modular `i32`/`u32` sums
carry an `i32x4` accumulator and fold its lanes once after the vector loop;
the original scalar seed is added once. Modular products still fold each
four-lane chunk before updating the scalar accumulator. A
vector loop handles complete chunks and uses the original scalar loop for the
remainder or a short trip. Simple unknown-alias loops use a nontrapping Wasm32
range and disjointness predicate and take the scalar path when it fails.
Each floating-point lane retains the original scalar evaluation order and
rounding; no relaxed SIMD or fused operation is introduced. Integer arithmetic
wraps modulo 2^32. Unsupported candidates remain scalar. Source `noalias`
contracts remain caller obligations.

The internal `WasmSliceRange` predicate uses widened unsigned 64-bit arithmetic.
It returns true for `count == 0`; otherwise it requires `start + count <=
slice.len` and the exclusive end byte address `slice.data + (start + count) *
element_bytes` to be no greater than both `2^32` and the current
`memory.size * 65536`. It is restricted to Wasm32 SIMD128 KIR, `u32` start and
count, and `i32`, `u32`, `i64`, `u64`, or `f64` slices with a matching element
width. This is an internal total range predicate, not a general checked-memory
mode. For the supported O3 `f64x2` affine loop shape, the independent checker
reconstructs the scalar addresses and exact range requirements, verifies
source `noalias` evidence and the original scalar fallback, and rejects a
candidate when those checks fail.

## Scalar address lowering

At O3, a bounded, independently checked direct-pointer loop can carry a
Wasm32 byte-address cursor when its sole changing loop state is a modular
integer induction variable and it directly accesses 4- or 8-byte primitives.
The cursor advances on the original backedge; unsupported loops retain their
ordinary address calculation. Both `baseline` and `simd128` profiles use this
scalar lowering.

For a struct-field access, CLI emission can place a constant field displacement
in a load memarg only when a verified alignment fact proves that the
original 32-bit address addition cannot wrap. Without that proof, it emits the
explicit addition. The memarg alignment is bounded by the actual field
alignment; it does not establish pointer validity, alias freedom, or an
additional runtime guard. Direct emission of an isolated KIR module has no
verified contract context and keeps the explicit addition.

## `slice<T>`

A `slice<T>` parameter is two collision-safe `i32` values in data,length order.
Internal calls and multi-value returns preserve the same order. A stored
descriptor is 8 bytes aligned to 4, with address at offset 0 and `u32` length at
offset 4. Address arithmetic uses the deterministic CK/WASM element layout.

WASM accepts `--overflow unchecked` and `--bounds unchecked`; either checked
selection is rejected before output. No implicit slice guard or trap is added.
The C/Native checked status ABI is not part of this ABI.

WebAssembly has no 0.15 runtime printing. A reachable print from an exported
root is rejected. An internal `main` does not create a WASI or browser entry.
