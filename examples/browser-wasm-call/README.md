# Browser WASM Pricing Example

[简体中文](README.zh-CN.md)

This example runs the WebAssembly module generated from `examples/pricing.ck`
in a browser. It is plain HTML and JavaScript: no framework, no bundler, and no
extra dependency.

## Generate WASM

From the repository root:

```sh
pnpm build
pnpm ckc emit-wasm examples/pricing.ck --out examples/browser-wasm-call/pricing.wasm
```

The example expects `pricing.wasm` next to `index.html` and `index.js`:

```text
examples/browser-wasm-call/
  index.html
  index.js
  pricing.wasm
```

## Serve Locally

Browsers usually cannot load WASM reliably from `file://`. Start a local HTTP
server instead:

```sh
cd examples/browser-wasm-call
python3 -m http.server 8000
```

Then open:

```text
http://localhost:8000/index.html
```

Click **Run pricing wasm**. The page prints the `calc_items` return code and
the computed output buffer.

## Browser ABI Notes

- `i64` / `u64` values use JavaScript `BigInt`.
- `ptr<T>` is a numeric offset into exported WASM memory.
- The example writes `Item` structs and reads the output buffer with
  `DataView`.
- WebAssembly memory is little-endian, so `DataView` calls pass `true` for
  little-endian reads and writes.
- WASM v1 does not do bounds checks.
- WASM v1 does not implement checked overflow.

The caller must choose valid memory offsets and allocate enough space for the
input `Item` array and output `i64` buffer.
