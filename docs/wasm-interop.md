# CK / CalcKernel WASM Interop Helper

`CKWasmArena` is a small JavaScript/TypeScript helper for host-side WASM memory
interop. It is not a CK / CalcKernel runtime, not a language runtime, and not a
GC, IO, strings, or allocator feature in generated CK code.

It helps JavaScript code manage:

- `WebAssembly.Memory`
- aligned byte offsets
- typed-array views over WASM memory
- bulk input copies with `TypedArray#set`
- explicit JS-owned output copies
- typed view refresh after `memory.grow`

## Import

```ts
import { CKWasmArena } from "calckernel";
```

When using the repository source directly in tests, import from
`src/wasm/ck-wasm-arena.js`.

## Heap Base

Current CK / CalcKernel WASM output exports `memory`, but it does not promise a
stable heap-base export. Do not guess a heap base.

Use one of these options:

```ts
const arena = new CKWasmArena(memory, { heapBase: 1024 });
```

or, when a module additively exports a heap base:

```ts
const arena = CKWasmArena.fromExports(instance.exports);
```

`fromExports` checks `__ck_heap_base` first, then `__heap_base`. Either export
may be a JavaScript number or `WebAssembly.Global`. If neither exists, allocation
methods throw until the caller supplies `heapBase`.

## Alignment

The helper allocates typed buffers with these alignments:

| Method | Alignment |
| --- | ---: |
| `allocF64` | 8 bytes |
| `allocI64` | 8 bytes |
| `allocU64` | 8 bytes |
| `allocI32` | 4 bytes |
| `allocU32` | 4 bytes |
| `allocBytes(bytes, align)` | caller-provided positive integer |

View methods validate pointer alignment before creating typed-array views.

## Recommended TypedArray Path

For large homogeneous buffers, prefer typed arrays over WASM memory:

```ts
const arena = new CKWasmArena(memory, { heapBase: 1024 });
const input = new Float64Array([1.0, 2.0, 3.0, 4.0]);
const { ptr, view } = arena.copyInF64(input);

view[1] = 2.5;
const outputView = arena.viewF64(ptr, input.length);
```

`copyInF64`, `copyInI32`, and `copyInU32` use `TypedArray#set`. They do not use
per-element `DataView` writes.

For `ptr<f64>`, `ptr<i32>`, and `ptr<u32>` buffers, the benchmark fast path
should prefer `viewF64`, `viewI32`, and `viewU32` over copying output back to a
new JavaScript array.

For pricing-style `i64` fixed-point buffers, prefer homogeneous SoA arrays over
mixed-width struct marshaling when the host can shape data that way:

```ck
export fn pricing_soa(
  prices: ptr<i64>,
  quantities: ptr<i64>,
  discounts: ptr<i64>,
  tax_rates_ppm: ptr<i64>,
  out_totals: ptr<i64>,
  n: i32
) -> i32
```

Use `viewI64`/`viewU64` and `BigInt64Array#set`/`BigUint64Array#set` for bulk
input copy. Keep `out_totals` as a WASM memory view when possible, and measure
any final output readback separately.

## Output View vs Copy

`viewF64(ptr, length)` returns a typed-array view over WASM memory. It is fast
and suitable when the caller can keep the result in WASM memory or consume it
before the next mutation.

`copyOutF64(ptr, length)` returns a JS-owned `Float64Array` copy:

```ts
const owned = arena.copyOutF64(ptr, length);
```

Use `copyOutF64` when the output must outlive later WASM memory writes, when it
must be transferred independently, or when caller code requires ordinary
JS-owned storage. It intentionally copies data.

For f64 performance work, the recommended shapes are:

- reductions such as `sum_f64`: copy input once with `Float64Array#set`, keep it
  resident in WASM memory, and consume the scalar `f64` return
- in-place/output kernels such as `axpy_f64`: keep immutable input resident
  where possible and keep the output as a WASM memory view by default
- pricing SoA kernels: bulk-copy `BigInt64Array` inputs once, keep them
  resident, and leave `ptr<i64>` output in WASM memory unless JS ownership is
  required
- copy-output paths: measure and use them separately because a JS-owned output
  copy can dominate total time

## Memory Growth

`memory.grow` can detach old typed-array views. `CKWasmArena` caches only the
current `memory.buffer`; view methods create fresh typed-array views from the
current buffer.

`ensureBytes(bytes)` grows memory when the current buffer is too small, then
refreshes the cached buffer. `refreshViewsIfNeeded()` can be called after host
code or foreign code grows memory.

Prefer pre-growing memory before a hot benchmark loop:

```ts
arena.ensureBytes(requiredBytes);
const values = arena.viewF64(ptr, length);
```

If memory grows during a loop, discard old typed-array views and request new
views from the arena.

## DataView Usage

`DataView` remains useful for:

- fallback and debug paths
- byte-exact ABI tests
- small buffers
- mixed-width struct packing, such as pricing `Item` layouts with multiple
  `i64` fields

Do not use per-element `DataView.setFloat64`, `DataView.getFloat64`,
`DataView.setBigInt64`, or `DataView.getBigInt64` in large benchmark hot paths
when a typed-array bulk path is available.

For pricing, the mixed-width/AoS `DataView` path is a fallback/debug ABI path.
The recommended benchmark path is SoA plus resident memory with typed-array
bulk copy. This does not mean every pricing workload is faster in WASM; it means
the benchmark can measure the CK / CalcKernel kernel instead of repeated
per-field host marshaling.

## Benchmark Interpretation

The helper reduces JS-WASM memory marshaling overhead, but it does not guarantee
that WASM is faster than JavaScript in every workload. JavaScript `Float64Array`
and `Int32Array` loops can be strong baselines, and total time depends on where
the data starts, whether input is copied, whether output is copied, and whether
memory grows during the measured loop.
For exact pricing arithmetic, JavaScript `BigInt` baselines and WASM `i64`
paths are different runtime models from JavaScript `Number` typed-array loops.
Do not claim WASM is always faster than JS; claim only the measured resident
SoA/low-copy shape.

Latest Phase 22 local full-run highlights on this machine, 2026-06-26:

| Workload | JS baseline | Recommended CK WASM path | DataView/copy fallback |
| --- | ---: | ---: | ---: |
| `f64-sum` | `f64-sum-js-float64array`: 112.375 ms | `f64-sum-ck-wasm-o3-optimized-low-copy-total`: 89.150 ms | `f64-sum-ck-wasm-o3-total`: 1033.140 ms |
| `f64-axpy` | `f64-axpy-js-float64array`: 114.551 ms | `f64-axpy-ck-wasm-o3-view-output-total`: 114.269 ms | `f64-axpy-ck-wasm-o3-copy-output-total`: 204.850 ms; `f64-axpy-ck-wasm-o3-total`: 1147.689 ms |
| `pricing` | `pricing-js-typedarray-number`: 123.519 ms | `pricing-wasm-soa-resident-total-O3`: 111.615 ms | `pricing-wasm-unchecked-total-O3`: 2562.414 ms |

These numbers are local benchmark results, not a portable guarantee. They
support the narrower claim that CK / CalcKernel WASM compute is competitive and
that SoA, resident memory, typed-array bulk copy, scalar returns, and output
views are the preferred interop shape when the workload can use them.

Benchmark summaries should continue to report:

- whether DataView participates in the hot path
- whether input is copied
- whether output is copied
- whether output is a JS-owned copy or WASM memory view
- whether memory grows in the benchmark
