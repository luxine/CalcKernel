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

```js
import { readFile } from 'node:fs/promises';
import { createCKWasmArena } from './examples/wasm/host/ck-wasm-arena.mjs';

const bytes = await readFile('build/pricing_batch.wasm');
const { instance } = await WebAssembly.instantiate(bytes);
const arena = createCKWasmArena(instance);

const count = 3;
const prices = arena.copyInI64(new BigInt64Array([100n, 200n, 300n]));
const quantities = arena.copyInI64(new BigInt64Array([2n, 1n, 4n]));
const discounts = arena.copyInI64(new BigInt64Array([5n, 0n, 20n]));
const taxRates = arena.copyInI64(new BigInt64Array([100_000n, 200_000n, 50_000n]));
const totals = arena.allocI64(count);

instance.exports.pricing_batch(
  prices.ptr, quantities.ptr, discounts.ptr, taxRates.ptr, totals, count,
);
const result = arena.copyOutI64(totals, count);
```

Use `allocI32`/`viewI32`, `allocU32`/`viewU32`, `allocI64`/`viewI64`,
`allocU64`/`viewU64`, or `allocF64`/`viewF64` for typed regions. `copyIn*`
allocates and copies a matching typed array; `copyOut*` returns a host-owned
copy. Views created before `memory.grow` become stale (and may be detached), so
request them again after any operation that can grow memory. The arena refreshes
its own view source after growth.

The example exports `pricing_one`, which computes one record, and
`pricing_batch`, which computes N structure-of-arrays records in one Wasm call.
Run the focused Node checks with `node --test examples/wasm/host/*.test.mjs`.
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
