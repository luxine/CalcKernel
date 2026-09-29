# WebAssembly Host Interop

[简体中文](../zh-CN/guides/wasm-interop.md)

Generate a module with:

```sh
ckc emit-wasm examples/wasm/scalar.ck --out build/scalar.wasm
```

The WebAssembly runtime owns instantiation; the caller owns memory contents.
Choose non-overlapping byte regions, write input values in little-endian form,
call the exported function, then read results. Use typed arrays for homogeneous
buffers and `DataView` for mixed-width structs or exact layout checks. Recreate
views after `memory.grow` because the backing buffer may change.

Pointers are `i32` byte offsets. A `ptr<f64>` index advances by 8 bytes. A
`slice<T>` argument is passed as address then `u32` length. Stored slice
descriptors use address at offset 0 and length at offset 4. The declared length
does not validate allocation extent; memory remains caller-owned and descriptors
may alias.

## Persistent host memory

`examples/wasm/host/ck-wasm-arena.mjs` provides a small monotonic allocator for
modules that export `memory` and `__ck_heap_base`. It keeps allocations across
calls, aligns typed regions, checks lengths and Wasm32 address arithmetic, and
grows memory when a requested range needs more pages. A failed growth leaves
the allocation cursor unchanged. The arena does not free memory or validate
other pointers; keep input and output regions within their allocations.

Build and run the complete host example with Node (WebAssembly support is built
into the runtime):

```sh
cargo build --release --locked --bin ckc
target/release/ckc emit-wasm examples/wasm/pricing_batch.ck \
  --out build/pricing_batch.wasm --opt-level 3 \
  --wasm-features baseline --overflow unchecked --bounds unchecked
node examples/wasm/host/pricing-batch.mjs build/pricing_batch.wasm
```

The example allocates the four input columns and output column once, forces one
memory growth to demonstrate invalidated views, reacquires each view from the
current memory buffer, and then updates and submits three batches through the
same arena. It prints the three result arrays as JSON. In an application, reserve
the expected workspace before the hot loop when practical; do not keep using a
typed array created before a possible `memory.grow`.

Use `allocI32`/`viewI32`, `allocU32`/`viewU32`, `allocI64`/`viewI64`,
`allocU64`/`viewU64`, or `allocF64`/`viewF64` for typed regions. `copyIn*`
allocates and copies a matching typed array; `copyOut*` returns a host-owned
copy. Views created before `memory.grow` become stale (and may be detached), so
request them again after any operation that can grow memory. The arena refreshes
its own view source after growth.

The example exports `pricing_one`, which computes one record, and
`pricing_batch`, which computes N structure-of-arrays records in one Wasm call.
The same directory contains the growth, alignment, invalid-length, and repeated
batch tests. Run the focused Node checks with
`node --test examples/wasm/host/*.test.mjs`.
For a timing comparison, build `ckc` and run:

```sh
cargo build --bin ckc
node benches/wasm/bench.mjs --ckc target/debug/ckc \
  --case pricing_one_calls --case pricing_batch_call --size 4096
```

The report checks every output and separates per-record calls from one batch
call. It also includes artifact bytes and host-side timing phases; a single
runtime's result does not establish a universal speedup.

WASM uses `--bounds unchecked` and `--overflow unchecked`. The CLI will reject
checked modes; no implicit trap or guard is inserted. Validate offsets and
lengths in the host when untrusted input reaches an export. See the normative
[WASM ABI](../abi/wasm.md).

The unchecked memory contract is separate from floating-point semantics: `f64`
operations preserve strict source evaluation order and binary64 rounding by
default. `baseline` is the default feature profile and emits no SIMD;
`--wasm-features simd128` is an explicit opt-in for the validated SIMD shapes in
the ABI. It does not enable Relaxed SIMD, FMA, fast math, or reassociation. A
source `requires noalias(a, b)` is a caller obligation: pass disjoint ranges of
the declared lengths even if the module itself accepts arbitrary pointer values.

For performance claims, distinguish a timed kernel invocation from host
preparation/readback and from first compile/instantiation. The repository's
Node/V8 runner measures CK WASM and checks its outputs; it does not provide
Clang/Rust WASM oracle timings. Native or historical Native performance reports
are not same-target WASM parity evidence. The recorded first module compile is
not browser cold start, and the per-round end-to-end phase uses an already
instantiated module and excludes memory growth. Keep those boundaries visible
when comparing implementations.
