# CK / CalcKernel WASM Pricing SoA Example

This example shows the recommended pricing interop shape:

- use structure-of-arrays instead of mixed-width AoS marshaling
- keep money as `i64` fixed-point integers
- use homogeneous `BigInt64Array` views over WASM memory
- keep output resident as a WASM memory view
- avoid `DataView` in the hot path

The `.ck` kernel uses integer arithmetic:

```ck
subtotal = price * quantity
after_discount = subtotal - discount
tax = after_discount * tax_rate_ppm / 1000000
total = after_discount + tax
```

Run from the repository root:

```sh
pnpm build
node examples/wasm/pricing-soa/run.mjs
```

Expected output:

```text
OK pricing-soa totals=20567,11000,6050,3078
```

This is an interop example, not a new language feature. It does not use `f64`
for financial amounts and does not recommend mixed-width struct packing with
per-field `DataView` writes for hot paths.
