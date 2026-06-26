# CK / CalcKernel WASM Interop Helper

`CKWasmArena` is a small JavaScript/TypeScript helper for host-side WASM memory
interop. It is not a CK / CalcKernel runtime, not a language runtime, and not a
GC, IO, strings, or allocator feature in generated CK code.

For release-facing WASM interop wording, see
[v0.7.0 release notes](releases/v0.7.0.md).

It helps JavaScript code manage:

- `WebAssembly.Memory`
- aligned byte offsets
- typed-array views over WASM memory
- bulk input copies with `TypedArray#set`
- explicit JS-owned output copies
- typed view refresh after `memory.grow`

## Import

```ts
import { CKWasmArena, createCKWasmArena } from "calckernel";
```

When using the repository source directly in tests, import from
`src/wasm/ck-wasm-arena.js`.

## API Stability

`CKWasmArena` is a public package-root export in the `calckernel` npm package.
For the v0.7.x release-hardening window it is considered experimental: the
current method names and behavior are intended to remain compatible across
v0.7.x, but minor changes may still happen before v1.0 if external consumer
testing finds a sharper API boundary.

Supported use cases:

- host-side aligned arena allocation for CK / CalcKernel WASM modules
- homogeneous `Float64Array`, `Int32Array`, `Uint32Array`, `BigInt64Array`, and
  `BigUint64Array` views over WASM memory
- bulk `copyInF64`, `copyInI32`, and `copyInU32` with `TypedArray#set`
- explicit `copyOutF64` when JS-owned output is required
- detecting `memory.grow` buffer changes before creating new views

Unsupported use cases:

- CK / CalcKernel runtime services, GC, IO, strings, or dynamic allocation
- automatic ownership or lifetime tracking for caller-created views
- automatic repair of old typed-array views after `memory.grow`
- pointer validity or bounds checks inside generated CK / CalcKernel kernels
- high-throughput mixed-width AoS marshaling with per-field `DataView`
- CommonJS `require("calckernel")`; the package is ESM-first, matching
  `package.json`

Errors thrown by the helper include the API name, the failure reason, and a
short repair hint. For example, constructor failures tell callers to pass
`instance.exports.memory`, and memory growth failures suggest pre-growing memory
or increasing the maximum.

## Public API Surface

The root package export exposes:

```ts
function createCKWasmArena(
  instanceOrExports: CKWasmInstanceLike | Record<string, unknown>,
  options?: CKWasmArenaOptions
): CKWasmArena;

class CKWasmArena {
  constructor(memory: CKWasmMemory, options?: CKWasmArenaOptions);

  static fromExports(exports: Record<string, unknown>, options?: CKWasmArenaOptions): CKWasmArena;
  static heapBaseFromExports(exports: Record<string, unknown>): number | undefined;

  ensureBytes(bytes: number): void;
  refreshViewsIfNeeded(): void;

  allocBytes(bytes: number, align: number): number;
  allocF64(length: number): number;
  allocI32(length: number): number;
  allocU32(length: number): number;
  allocI64(length: number): number;
  allocU64(length: number): number;

  viewF64(ptr: number, length: number): Float64Array;
  viewI32(ptr: number, length: number): Int32Array;
  viewU32(ptr: number, length: number): Uint32Array;
  viewI64(ptr: number, length: number): BigInt64Array;
  viewU64(ptr: number, length: number): BigUint64Array;

  copyInF64(src: Float64Array): CKWasmArenaCopy<Float64Array>;
  copyInI32(src: Int32Array): CKWasmArenaCopy<Int32Array>;
  copyInU32(src: Uint32Array): CKWasmArenaCopy<Uint32Array>;
  copyOutF64(ptr: number, length: number): Float64Array;
}
```

Type declarations are emitted under `dist/src`, and external consumers should
import from the package root:

```ts
import { CKWasmArena, createCKWasmArena, type CKWasmArenaOptions } from "calckernel";
```

`CKWasmMemory` is the package's public structural type for the `buffer` and
`grow` members needed by TypeScript consumers in Node-oriented projects. At
runtime, `CKWasmArena` still validates that the value is a real
`WebAssembly.Memory` instance and rejects lookalike objects.

## Memory Ownership

Current CK / CalcKernel WASM output owns and exports one linear memory:

```wat
(memory (export "memory") 1)
(global (export "__ck_heap_base") i32 (i32.const 0))
```

The compiler does not currently emit imported memory, multiple memories, data
segments, or static storage. `__ck_heap_base` is additive interop metadata: it
does not change the existing `memory` export, function exports, pointer byte
offset ABI, or `f64` scalar ABI.

Host JavaScript owns memory placement above the heap base. `CKWasmArena` can
grow the exported memory and choose aligned offsets, but it is not a runtime
allocator in generated CK code. Generated kernels still trust the pointers and
lengths supplied by the host.

Imported memory is not emitted by the current backend. If a future or custom
module imports memory, the helper can still be used only when the instantiated
module exposes that memory as `exports.memory` or the caller passes the actual
`WebAssembly.Memory` to `new CKWasmArena(memory, { heapBase })`.

## Heap Base

Do not guess a heap base. Use the documented resolution order:

1. explicit `options.heapBase`
2. `instance.exports.__ck_heap_base`
3. `instance.exports.__heap_base`
4. otherwise throw a clear error asking for `heapBase`

The recommended flow for CK / CalcKernel-generated WASM is:

```ts
const { instance } = await WebAssembly.instantiate(bytes);
const arena = createCKWasmArena(instance);
```

Use the explicit flow for custom modules, older generated artifacts, or tests
that deliberately manage memory layout:

```ts
const arena = new CKWasmArena(memory, { heapBase: 1024 });
```

`createCKWasmArena(instanceOrExports, options)` accepts a
`WebAssembly.Instance`-like object or an exports object, verifies
`exports.memory`, resolves the heap base, and returns a ready-to-allocate
`CKWasmArena`. `CKWasmArena.fromExports(exports, options)` remains available
for lower-level callers; allocation still requires an explicit or exported heap
base.

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
const arena = createCKWasmArena(instance);
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

The helper does not and cannot repair typed-array views that user code already
cached. If memory grows, discard old user-held views and request new views from
the arena.

Prefer pre-growing memory before a hot benchmark loop:

```ts
arena.ensureBytes(requiredBytes);
const values = arena.viewF64(ptr, length);
```

If memory grows during a loop, discard old typed-array views and request new
views from the arena.

## Official Examples

The release-hardened examples under `examples/wasm` are the recommended
copy-and-run interop entry points:

```sh
pnpm build
node examples/wasm/f64-sum/run.mjs
node examples/wasm/f64-axpy/run.mjs
node examples/wasm/pricing-soa/run.mjs
```

- [`examples/wasm/f64-sum`](../examples/wasm/f64-sum/README.md) copies
  `Float64Array` input with `copyInF64`, calls a CK / CalcKernel WASM kernel,
  and consumes the scalar `f64` return. There is no output readback.
- [`examples/wasm/f64-axpy`](../examples/wasm/f64-axpy/README.md) keeps input
  and output in WASM memory, uses `viewF64` as the default output path, and
  demonstrates `copyOutF64` only as an explicit JS-owned copy.
- [`examples/wasm/pricing-soa`](../examples/wasm/pricing-soa/README.md) uses
  integer fixed-point pricing data in homogeneous SoA `BigInt64Array` buffers.
  It avoids `f64` for financial amounts and avoids mixed-width AoS `DataView`
  marshaling as the recommended path.

These examples compile their local `.ck` source with `ckc emit-wasm`, create a
`CKWasmArena` with `createCKWasmArena(instance)`, and write generated WASM under
`build/examples/wasm`. They are correctness examples, not benchmark thresholds,
and they intentionally keep `DataView` out of the hot path.

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

The helper reduces JS-WASM memory marshaling overhead, but it does not turn
WASM into a blanket replacement for JavaScript hot loops. JavaScript
`Float64Array` and `Int32Array` loops can be strong baselines, and total time
depends on where the data starts, whether input is copied, whether output is
copied, and whether memory grows during the measured loop.
For exact pricing arithmetic, JavaScript `BigInt` baselines and WASM `i64`
paths are different runtime models from JavaScript `Number` typed-array loops.
Keep claims scoped to the measured resident SoA/low-copy shape.

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
