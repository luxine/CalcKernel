# Python ctypes Pricing Example

This example calls the dynamic library generated from `examples/pricing.ik`
using only Python's standard `ctypes` module.

## Build the Dynamic Library

From the repository root, build the TypeScript CLI first:

```sh
pnpm build
```

Then generate and compile the pricing dynamic library.

On macOS:

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing
```

This creates:

```text
build/libpricing.dylib
```

On Linux:

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing
```

This creates:

```text
build/libpricing.so
```

On Windows:

```sh
pnpm ikc build examples/pricing.ik --out build/pricing.dll
```

This creates:

```text
build/pricing.dll
```

## Run the Example

From the repository root:

```sh
python3 examples/python-ctypes-call/call_pricing.py
```

On Windows:

```sh
py examples\python-ctypes-call\call_pricing.py
```

The script prints `OK` when `calc_items` returns `0` and the output buffer
matches the expected values.

## Checked Mode

Checked mode has a different C ABI. The function returns `IK_Status`, and the
original IntKernel return value is written through a final pointer argument.

Build the checked dynamic library from the repository root.

On macOS:

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
```

This creates:

```text
build/libpricing_checked.dylib
```

On Linux:

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
```

This creates:

```text
build/libpricing_checked.so
```

On Windows, pass the desired `.dll` file name explicitly:

```sh
pnpm ikc build examples/pricing.ik --out build/pricing_checked.dll --overflow checked
```

This creates:

```text
build/pricing_checked.dll
```

Run the checked example:

```sh
python3 examples/python-ctypes-call/call_pricing_checked.py
```

On Windows:

```sh
py examples\python-ctypes-call\call_pricing_checked.py
```

The script prints `OK` for the successful pricing call and `overflow check OK`
for a case where `price * qty` overflows.

## ctypes Mapping

The generated C header defines:

```c
typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;

IK_API int32_t calc_items(Item* items, int32_t len, int64_t* out);
```

The Python binding mirrors this exactly:

- `i64` maps to `ctypes.c_int64`
- `i32` maps to `ctypes.c_int32`
- `ptr<Item>` maps to an array or pointer of `Item`
- `ptr<i64>` maps to an array or pointer of `ctypes.c_int64`

The caller allocates both buffers:

- `items = (Item * n)(...)`
- `out = (ctypes.c_int64 * n)(...)`

V0 does not allocate memory, free memory, perform bounds checks, or check integer
overflow. The caller must pass valid pointers, valid lengths, and values that
stay within the intended integer ranges.

## Checked ctypes Mapping

The checked generated C header defines status values:

```c
typedef int32_t IK_Status;

#define IK_OK ((IK_Status)0)
#define IK_ERR_OVERFLOW ((IK_Status)1)
#define IK_ERR_DIV_BY_ZERO ((IK_Status)2)
#define IK_ERR_NULL_POINTER ((IK_Status)3)
```

The checked `calc_items` declaration is:

```c
IK_API IK_Status calc_items(Item* items, int32_t len, int64_t* out, int32_t* ik_return);
```

The Python binding maps this as:

```python
IK_Status = ctypes.c_int32
IK_OK = 0
IK_ERR_OVERFLOW = 1
IK_ERR_DIV_BY_ZERO = 2
IK_ERR_NULL_POINTER = 3

lib.calc_items.argtypes = [
    ctypes.POINTER(Item),
    ctypes.c_int32,
    ctypes.POINTER(ctypes.c_int64),
    ctypes.POINTER(ctypes.c_int32),
]
lib.calc_items.restype = ctypes.c_int32
```

The caller still allocates `items`, `out`, and `ik_return`:

```python
out = (ctypes.c_int64 * len(items))(0, 0, 0)
ik_return = ctypes.c_int32()
status = lib.calc_items(items, ctypes.c_int32(len(items)), out, ctypes.byref(ik_return))
```

Checked mode checks integer overflow, division by zero, and the generated
`ik_return` pointer. It still does not perform bounds checks, validate user
data pointers, or verify that output buffers are large enough.
