# Node.js WASM f64 Float64Array Example

[简体中文](README.zh-CN.md)

This example calls an CK / CalcKernel WASM `f64` kernel from Node.js with a
`Float64Array` view over exported WASM memory. It is the recommended host pattern
for batched `ptr<f64>` buffers when the data source is already numeric and
8-byte aligned.

The example does not add an CK runtime or allocator. The host chooses byte
offsets, grows memory when needed, and rebuilds typed-array views after growth.

## Generate WASM

Build the local CLI from the repository root:

```sh
pnpm build
```

Generate `build/f64_array.wasm`:

```sh
ckc emit-wasm examples/node-wasm-f64-array/f64_array.ck --out build/f64_array.wasm -O3
```

In a source checkout, you can run the same CLI through pnpm:

```sh
pnpm ckc emit-wasm examples/node-wasm-f64-array/f64_array.ck --out build/f64_array.wasm -O3
```

## Run

From the repository root:

```sh
node examples/node-wasm-f64-array/index.mjs
```

Or pass a custom WASM path:

```sh
node examples/node-wasm-f64-array/index.mjs --wasm build/f64_array.wasm
```

The script prints `OK` when the `axpy_f64` output buffer matches the expected
values within tolerance.

## ptr<f64> Rules

WASM `ptr<f64>` values are `i32` byte offsets:

- `f64` size is 8 bytes.
- `ptr<f64>[i]` uses byte offset `base + i * 8`.
- `Float64Array` index is `byteOffset / 8`.
- `byteOffset` must be 8-byte aligned.

The hot path uses `Float64Array#set` and `Float64Array#subarray` instead of
per-element `DataView.setFloat64` / `DataView.getFloat64`.

`DataView` remains useful for byte-level ABI tests and mixed-width struct
packing. `Float64Array` is the bulk path for homogeneous `f64` buffers.

## Memory Ownership

CK / CalcKernel currently provides no WASM allocator, runtime, or bounds checks.
The host owns memory placement and must ensure input/output regions are valid.

If `memory.grow` runs, previous `Float64Array` views may be detached. Recreate
typed-array views after any growth:

```js
const values = new Float64Array(memory.buffer);
```

Do not assume this path is always faster than `DataView`; it depends on where
the data comes from and how much copying the host already needs. It is the
recommended baseline for large homogeneous `f64` arrays.
