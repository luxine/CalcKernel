# Node.js FFI Pricing Example

[简体中文](README.zh-CN.md)

This example calls the dynamic library generated from `examples/pricing.ck`
through Node.js. It is intentionally isolated from the root project so the main
compiler package does not depend on a native FFI module.

The example uses `koffi`, a lightweight Node.js C FFI package, only inside this
directory.

## Build the Dynamic Library

From the repository root, build the CalcKernel CLI:

```sh
pnpm build
```

Then compile the pricing kernel.

On macOS:

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing
```

This creates:

```text
build/libpricing.dylib
```

On Linux:

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing
```

This creates:

```text
build/libpricing.so
```

On Windows:

```sh
pnpm ckc build examples/pricing.ck --out build/pricing.dll
```

This creates:

```text
build/pricing.dll
```

## Install and Run

Install the example dependency from inside this directory:

```sh
cd examples/node-ffi-call
pnpm install
pnpm start
```

You can also use npm:

```sh
cd examples/node-ffi-call
npm install
npm start
```

The script prints `OK` when `calc_items` returns `0` and the output buffer
matches the expected values.

## Checked Mode

Checked mode has a different C ABI. The function returns `CK_Status`, and the
original CalcKernel return value is written through a final output pointer.

Build the checked dynamic library from the repository root.

On macOS:

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
```

This creates:

```text
build/libpricing_checked.dylib
```

On Linux:

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
```

This creates:

```text
build/libpricing_checked.so
```

On Windows, pass the desired `.dll` file name explicitly:

```sh
pnpm ckc build examples/pricing.ck --out build/pricing_checked.dll --overflow checked
```

This creates:

```text
build/pricing_checked.dll
```

Run the checked example from this directory:

```sh
pnpm start:checked
```

Or directly:

```sh
node checked.mjs
```

The script prints `OK` after a successful call and an overflow case where
`price * qty` returns `CK_ERR_OVERFLOW`.

## FFI Mapping

The generated C header defines:

```c
typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;

CK_API int32_t calc_items(Item* items, int32_t len, int64_t* out);
```

The Node.js binding mirrors this with Koffi:

- `i64` / `int64_t` maps to Koffi `int64_t`; this example passes values as
  `BigInt`.
- `i32` / `int32_t` maps to Koffi `int32_t`; JavaScript `number` is used for
  small 32-bit values such as `len`.
- `struct Item` maps to `koffi.struct("Item", { ... })`.
- `ptr<Item>` is passed as a JavaScript array of `Item`-shaped objects.
- `ptr<i64>` is passed as a caller-owned `BigInt64Array` output buffer.

## Checked FFI Mapping

The checked generated header defines status values:

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
```

The checked `calc_items` declaration is:

```c
CK_API CK_Status calc_items(Item* items, int32_t len, int64_t* out, int32_t* ck_return);
```

The Koffi signature mirrors that ABI:

```js
const calcItems = lib.func("int32_t calc_items(Item *items, int32_t len, _Out_ int64_t *out, _Out_ int32_t *ck_return)");
```

The checked example uses:

- `number` for `CK_Status`, `CK_OK`, and other 32-bit status constants.
- `Int32Array(1)` for the final `ck_return` pointer.
- `BigInt` values for every `int64_t` field in `Item`.
- `BigInt64Array` for the `int64_t* out` buffer.

This avoids silently converting `i64` values into unsafe JavaScript `number`
values. Keep using `BigInt` or Koffi-supported 64-bit representations for
`i64` / `u64` values in real integrations.

## V0 Safety Notes

V0 does not allocate memory, free memory, or perform bounds checks. Checked mode
checks integer overflow, division by zero, and the generated `ck_return`
pointer, but it still does not validate user data pointers or buffer lengths.
The caller must pass valid buffers and a length that matches the allocated
arrays. If the length is wrong, the native code has the same risks as equivalent
C pointer indexing.
