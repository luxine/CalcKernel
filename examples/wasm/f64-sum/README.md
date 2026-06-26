# CK / CalcKernel WASM f64 Sum Example

This example shows the recommended read-only/scalar-return WASM interop shape:

- compile a `.ck` kernel with `ckc`
- create `CKWasmArena` from the generated instance
- copy input with `Float64Array#set` through `copyInF64`
- call a WASM kernel that returns scalar `f64`
- avoid output readback because the result is the scalar return

Run from the repository root:

```sh
pnpm build
node examples/wasm/f64-sum/run.mjs
```

Expected output:

```text
OK f64-sum result=17 inputLength=5
```

This path does not use `DataView` in the hot path. It is appropriate for
read-only `ptr<f64>` inputs and scalar-return reductions.
