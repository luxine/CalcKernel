# CK / CalcKernel WASM f64 Axpy Example

This example shows the recommended output-view interop shape for a kernel that
writes `ptr<f64>` output:

- keep `x` and `y` in WASM memory
- call the CK / CalcKernel WASM export
- use `viewF64` as the default fast output path
- use `copyOutF64` only when JS-owned output is explicitly required
- pre-grow memory before creating hot-path views

Run from the repository root:

```sh
pnpm build
node examples/wasm/f64-axpy/run.mjs
```

Expected output:

```text
OK f64-axpy checksum=49.5 output=2.5,3,16,28
```

`memory.grow` can detach old typed-array views. This runner calls
`arena.ensureBytes(...)` before creating the working views; if user code grows
memory later, discard old views and request fresh ones from `CKWasmArena`.

This path does not use `DataView` in the hot path. The default output is a WASM
memory view; `copyOutF64` is shown as an explicit copy path.
