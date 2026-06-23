# Node.js WASM Pricing Example

[简体中文](README.zh-CN.md)

This example calls the WebAssembly module generated from `examples/pricing.ik`
with the built-in Node.js WebAssembly API. It does not need a native
`.so`, `.dylib`, or `.dll`, and it does not install any example-local
dependency.

## Generate WASM

Build the local CLI from the repository root:

```sh
pnpm build
```

From this example directory, generate `../../build/pricing.wasm`:

```sh
ikc emit-wasm ../../examples/pricing.ik --out ../../build/pricing.wasm
```

In a source checkout, you can run the same CLI through pnpm from this directory:

```sh
pnpm --dir ../.. ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
```

## Run

From the repository root:

```sh
node examples/node-wasm-call/index.mjs
```

Or from this directory:

```sh
node index.mjs
```

The script prints `OK` when `calc_items` returns `0` and the output buffer
matches the expected values.

## ABI Mapping

The generated WASM exports:

- `memory`: one WebAssembly linear memory.
- `calc_items(items: i32, len: i32, out: i32) -> i32`.

`ptr<T>` is a numeric memory offset. The host allocates regions manually by
choosing offsets inside `memory.buffer`.

`i64` and `u64` values use JavaScript `BigInt`. Do not pass large 64-bit values
as JavaScript `number`.

The host writes memory with `DataView`. WebAssembly memory is little-endian, so
all `DataView` reads and writes in this example pass `true` for the
`littleEndian` parameter.

## Item Layout

`pricing.ik` defines:

```ik
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}
```

WASM layout:

| Field | Offset | Type |
| --- | ---: | --- |
| `price` | 0 | `i64` |
| `qty` | 8 | `i64` |
| `discount` | 16 | `i64` |
| `tax_rate_ppm` | 24 | `i64` |

`sizeof(Item) = 32`.

The caller allocates both the input `Item` array and the output `i64` buffer.

## Safety Notes

WASM v1 is unchecked:

- no bounds check
- no checked overflow
- no pointer validity check
- no allocator
- no runtime

The caller must ensure that `items`, `len`, and `out` describe valid memory
regions.
