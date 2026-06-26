# CK / CalcKernel WASM ABI

[简体中文](zh-CN/WASM_ABI.md)

This document defines the Phase 12 WebAssembly ABI for CalcKernel. The current
Phase 12 implementation can emit WAT and WASM for unchecked scalar operations,
control flow, internal function calls, logical short-circuiting, and core
`ptr<T>` memory load/store patterns.

## Phase 12 Goal

Phase 12 adds a WASM backend after MIR:

```text
.ck source
  -> lexer
  -> parser
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> MIR-to-WAT backend
  -> .wat
  -> WAT-to-WASM assembly through the `wabt` npm package
  -> .wasm
```

Current CLI commands:

```sh
ckc emit-wat examples/scalar.ck --out build/scalar.wat
ckc emit-wat examples/scalar.ck
ckc emit-wasm examples/scalar.ck --out build/scalar.wasm
ckc emit-wasm examples/pricing.ck --out build/pricing.wasm
```

`emit-wat` can write to a file or stdout. `emit-wasm` writes binary output and
therefore requires `--out`.

The Phase 12 v1 backend targets `wasm32`, consumes validated MIR, and keeps the
same no-runtime and caller-owned-memory model as the C backend.

`emit-wasm` compiles generated WAT with the `wabt` npm package bundled as an
CalcKernel runtime dependency. No external `wat2wasm` executable is required for
the packaged CLI.

## WASM v1 Scope

Supported by the Phase 12 v1 design:

- `i32`
- `i64`
- `u32`
- `u64`
- `f64`
- `bool`
- `ptr<T>`
- deterministic struct memory layout
- exported functions
- non-exported internal functions
- exported linear memory
- unchecked arithmetic
- `f64` arithmetic, comparison, load, and store
- exact explicit `i32_to_f64` / `u32_to_f64` casts
- WAT-to-WASM assembly through `wabt`

WASM floating point follows the project-wide f64-only policy: `f64` is the only
floating point type, `f32` is not planned, implicit int/float conversion is not
supported, and only exact explicit `i32_to_f64` / `u32_to_f64` casts are
available. Scalar f64 interop uses JavaScript `Number`.

Not supported by Phase 12 v1:

- checked overflow
- WASI
- imports
- heap allocation
- strings
- bounds check
- `slice<T>`
- runtime library
- SIMD
- threads
- GC
- exceptions

The current `emit-wasm` implementation supports `ptr<T>` load/store codegen for
MIR places such as `items[i].price` and `out[i] = value`. It still does not add
bounds checks or pointer validity checks.

Phase 16.9 supports `f64` scalar arithmetic, comparison, load, and store
codegen. `f64` has size 8, alignment 8, scalar ABI type `f64`, and host
JavaScript interop uses `Number`, not `BigInt`.

## Type Mapping

| CalcKernel type | WASM value type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `bool` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `f64` | `f64` |
| `ptr<T>` | `i32` memory offset |

WASM does not have distinct `u32` or `u64` value types. Signedness is selected
by instruction choice for division, remainder, and comparisons.

`bool` uses `i32`: `0` is false, and any nonzero value is true. Codegen should
produce canonical `0` or `1` results for CalcKernel boolean expressions.

JavaScript's WebAssembly API represents `i64` and `u64` parameters and return
values as `BigInt`. It represents `f64` parameters and return values as
JavaScript `Number`.

F64 semantics are intentionally strict and ordinary for WebAssembly:

- arithmetic uses `f64.add`, `f64.sub`, `f64.mul`, `f64.div`, and `f64.neg`
- comparisons use `f64.eq`, `f64.ne`, `f64.lt`, `f64.le`, `f64.gt`, and
  `f64.ge`
- memory uses `f64.load` and `f64.store`
- NaN, infinity, and `-0.0` follow WebAssembly f64 behavior
- host tests must use `Number.isNaN`, signed infinity checks, `Object.is` for
  `-0`, and tolerance for finite values instead of bit equality
- `f64` is never passed as `BigInt`

## Function ABI

Exported CalcKernel functions are exported from the WASM module with their
source names.

Example CalcKernel:

```ck
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

Target WAT shape:

```wat
(func $add_i64 (export "add_i64")
  (param $a i64)
  (param $b i64)
  (result i64)
  ;; implementation
)
```

Boolean returns use `i32`:

```ck
export fn is_positive(a: i64) -> bool {
  return a > 0;
}
```

Target WASM result:

```wat
(result i32)
```

Non-exported CalcKernel functions are emitted as internal WASM functions without
an export entry.

## Pointer ABI

`ptr<T>` is a `wasm32` linear-memory offset represented as an `i32`.
`ptr<f64>` is still an `i32` byte offset; indexing advances by 8 bytes per
element.

Example CalcKernel:

```ck
export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
  ...
}
```

Target WASM ABI:

```wat
(param $items i32)
(param $len i32)
(param $out i32)
(result i32)
```

The caller is responsible for:

- writing the `Item` array into WASM memory
- passing the `items` memory offset
- passing `len`
- reserving an output buffer in WASM memory
- passing the `out` memory offset
- reading the output buffer after the call

The compiler does not own, allocate, grow, or validate these buffers.

## Memory

Phase 12 v1 generates one exported memory:

```wat
(memory (export "memory") 1)
```

One WebAssembly page is 64 KiB.

Phase 12 v1 does not provide an allocator. The host manually chooses offsets in
linear memory. The backend does not generate a `memory.grow` helper. A future
phase may add a simple allocator, but Phase 12 does not.

## Struct Layout

WASM uses a deterministic CalcKernel-defined layout, independent of the host C
compiler. This does not change the C ABI layout: generated C headers and C
harnesses continue to use the target C compiler's normal struct layout rules.

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

Struct layout rules:

- fields are laid out in declaration order
- each field offset is aligned to the field alignment
- padding is inserted when needed
- struct alignment is the maximum field alignment
- struct size is padded to the struct alignment

Example:

```ck
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}
```

Layout:

| Field | Offset |
| --- | ---: |
| `price` | 0 |
| `qty` | 8 |
| `discount` | 16 |
| `tax_rate_ppm` | 24 |

`sizeof(Item) = 32`

`align(Item) = 8`

## Load and Store Mapping

`items[i].price` lowers to address arithmetic plus a load:

```text
address = items + i * sizeof(Item) + offset(price)
i64.load address
```

`out[i] = value` lowers to address arithmetic plus a store:

```text
address = out + i * sizeof(i64)
i64.store address value
```

The backend chooses the load/store instruction from the value type:

- `i32.load` / `i32.store` for `i32`, `u32`, `bool`, and `ptr<T>`
- `i64.load` / `i64.store` for `i64` and `u64`
- `f64.load` / `f64.store` for `f64`

All host-side examples should use little-endian reads and writes. WebAssembly
linear memory is little-endian. Host tests and harnesses should use
`DataView.getFloat64(offset, true)` and `DataView.setFloat64(offset, value,
true)` for `f64` buffers.

## Arithmetic Mapping

Phase 12 v1 is unchecked.

For addition, subtraction, and multiplication, signed and unsigned integer
types use the same WASM arithmetic instruction for a given width:

- `i32.add`, `i32.sub`, `i32.mul`
- `i64.add`, `i64.sub`, `i64.mul`
- `f64.add`, `f64.sub`, `f64.mul`

Division and remainder must choose signed or unsigned instructions:

- `i32.div_s` / `i32.div_u`
- `i64.div_s` / `i64.div_u`
- `i32.rem_s` / `i32.rem_u`
- `i64.rem_s` / `i64.rem_u`

F64 division uses `f64.div`. F64 remainder is not supported. Unary `-f64`
lowers to `f64.neg`; integer negation keeps the existing zero-subtraction
lowering.

Comparisons must also choose signed or unsigned instructions:

- `i32.lt_s` / `i32.lt_u`
- `i32.le_s` / `i32.le_u`
- `i32.gt_s` / `i32.gt_u`
- `i32.ge_s` / `i32.ge_u`
- `i64.lt_s` / `i64.lt_u`
- `i64.le_s` / `i64.le_u`
- `i64.gt_s` / `i64.gt_u`
- `i64.ge_s` / `i64.ge_u`

Equality comparisons use the same instruction regardless of signedness:

- `i32.eq`, `i32.ne`
- `i64.eq`, `i64.ne`

F64 comparisons use the standard WASM f64 predicates:

- `f64.eq`, `f64.ne`
- `f64.lt`, `f64.le`
- `f64.gt`, `f64.ge`

Phase 20 supports explicit int-to-f64 casts:

- `i32_to_f64(x)` lowers to `f64.convert_i32_s`.
- `u32_to_f64(x)` lowers to `f64.convert_i32_u`.
- `i64_to_f64`, `u64_to_f64`, f64-to-int casts, and implicit conversion remain
  unsupported.
- Cast results are ordinary WASM `f64` values and use JavaScript `Number` at
  the host boundary.

## Checked Overflow

Phase 12 v1 does not support checked WASM code generation. The WASM backend is
unchecked-only in this phase and must reject `--overflow checked` instead of
silently generating unchecked output.

If a user runs:

```sh
ckc emit-wat input.ck --overflow checked
ckc emit-wasm input.ck --overflow checked
```

the compiler must report:

```text
error: WASM backend does not support --overflow checked yet.
help: use --overflow unchecked, or use emit-c/build for checked C output.
```

Use the C backend (`emit-c` or `build`) when checked arithmetic is required. WASM
checked arithmetic requires explicit overflow-check lowering for WASM
instructions and is future work.

## Node.js Interop

Node.js can instantiate generated WASM with the built-in WebAssembly API:

```js
const bytes = await fs.promises.readFile("build/pricing.wasm");
const { instance } = await WebAssembly.instantiate(bytes);
```

Interop rules:

- use `instance.exports.memory` to access linear memory
- use `DataView` or typed arrays to write input buffers and read output buffers
- use little-endian `DataView` methods
- pass `ptr<T>` values as numeric memory offsets
- pass and receive `i64` / `u64` values as `BigInt`
- pass and receive `f64` values as JavaScript `Number`
- interpret `bool` results as `result !== 0`

The host is responsible for choosing non-overlapping memory regions for input
and output buffers.

Example host-side `Item` writer for the layout above:

```js
const memory = instance.exports.memory;
const view = new DataView(memory.buffer);
const ITEM_SIZE = 32;

function writeItem(offset, item) {
  view.setBigInt64(offset + 0, item.price, true);
  view.setBigInt64(offset + 8, item.qty, true);
  view.setBigInt64(offset + 16, item.discount, true);
  view.setBigInt64(offset + 24, item.taxRatePpm, true);
}

writeItem(0, {
  price: 1234n,
  qty: 2n,
  discount: 0n,
  taxRatePpm: 100000n
});

// A ptr<Item> argument is just the numeric offset.
const price = instance.exports.first_price(0);
```

For `ptr<f64>` buffers, `DataView` is useful for byte-level ABI tests and
mixed-width layout checks:

```js
view.setFloat64(valuesOffset + 8, 2.5, true);
const value = view.getFloat64(valuesOffset + 8, true);
```

For large homogeneous `f64` arrays, prefer a `Float64Array` view over exported
WASM memory:

```js
const memory = instance.exports.memory;
const values = new Float64Array(memory.buffer);
const xOffset = 0;
const yOffset = 64;
const xIndex = xOffset / 8;
const yIndex = yOffset / 8;

values.set([1.0, 2.0, 3.0, 4.0], xIndex);
values.set([0.5, 1.25, 1.25, 2.0], yIndex);

const checksum = instance.exports.axpy_f64(1.25, xOffset, yOffset, 4);
const y = values.subarray(yIndex, yIndex + 4);
```

The same pointer rules still apply:

- WASM `ptr<f64>` is an `i32` byte offset.
- `f64` size is 8 bytes.
- `ptr<f64>[i]` uses byte offset `base + i * 8`.
- `Float64Array` index is `byteOffset / 8`.
- `byteOffset` must be 8-byte aligned.

If host code calls `memory.grow`, old `Float64Array` views may be detached.
Create the view after growth, and recreate it after any later growth. CK does
not provide an allocator or runtime; host code still owns memory placement and
buffer sizing. This is a low-copy host pattern, not a promise that every input
source is zero-copy: if data starts outside WASM memory, the host still pays to
place it there. Keep `DataView` for byte-exact ABI tests, mixed-width structs,
and layout debugging. See `examples/node-wasm-f64-array/` for a full Node.js
example.

## Browser Interop

Generated WASM can also run in browsers through the standard WebAssembly API.
The browser example in `examples/browser-wasm-call/` uses:

- `fetch("./pricing.wasm")`
- `WebAssembly.instantiateStreaming` when available
- an `arrayBuffer` + `WebAssembly.instantiate` fallback
- `DataView` for little-endian memory writes and reads
- numeric memory offsets for `ptr<T>` values
- `BigInt` for any `i64` / `u64` values crossing the JS/WASM boundary
- `Number` for any `f64` values crossing the JS/WASM boundary

Browsers usually cannot fetch `.wasm` from `file://`. Serve the example through
a local HTTP server, for example:

```sh
pnpm ckc emit-wasm examples/pricing.ck --out examples/browser-wasm-call/pricing.wasm
cd examples/browser-wasm-call
python3 -m http.server 8000
```

Then open `http://localhost:8000/index.html`.

## Benchmark Notes

The WASM benchmark in `bench/wasm_pricing_benchmark.mjs` compares the same
batched `pricing.ck` workload shape as the JavaScript and C benchmark harnesses.
Generate the module first:

```sh
pnpm ckc emit-wasm examples/pricing.ck --out build/pricing.wasm --overflow unchecked
node bench/wasm_pricing_benchmark.mjs
```

The benchmark writes large `Item` arrays into exported memory, calls
`calc_items(itemsOffset, len, outOffset)`, and reads the output buffer back with
`DataView`. For large inputs it may call `memory.grow` on the host side. That is
benchmark setup code only; CalcKernel V0 does not provide an allocator or a
runtime memory-growth helper.

Benchmark results are rough local references. They do not prove checked
arithmetic safety. The Phase 12 WASM backend is unchecked-only.

## No Bounds Check in Phase 12

Phase 12 v1 does not add bounds checks.

Reasons:

- `ptr<T>` does not carry a length
- the compiler does not know the length of `out` buffers
- the compiler cannot validate arbitrary host-provided memory offsets
- the compiler does not validate pointer offsets or pointer lifetimes

Caller responsibilities:

- pass valid memory offsets
- ensure `items` points to at least `len` `Item` values when a function reads
  `items[i]`
- ensure `out` points to enough writable memory when a function writes `out[i]`
- avoid overlapping buffers unless the specific kernel is written to tolerate
  that aliasing

Future bounds checking should wait for a length-carrying type such as
`slice<T>` or explicit pointer-plus-length metadata.
