# CalcKernel C ABI

[简体中文](zh-CN/ABI.md)

CalcKernel V0 targets a plain C ABI. Generated `.h` and `.c` files are intended
to be compiled by a C compiler and consumed from C, C++, Python, Node.js, Rust,
Go, C#, and other host languages through their normal FFI mechanisms.

V0 does not provide a runtime. It does not allocate memory, free memory, or own
the lifetime of any pointer passed across the ABI. The caller owns every input
and output buffer.

## Cross-Backend f64 Summary

Phase 16 f64 strict mode uses these ABI mappings:

| Backend | Scalar `f64` | `ptr<f64>` | Struct field `f64` |
| --- | --- | --- | --- |
| C | `double` | `double*` | C `double` field |
| LLVM | `double` | opaque `ptr` with `getelementptr double` | LLVM `double` field |
| WASM | `f64` value type | `i32` byte offset, 8-byte element step | deterministic size 8 / align 8 field |

JavaScript WASM interop uses `Number` for f64 parameters and returns, not
`BigInt`. WASM host memory access for `ptr<f64>` uses little-endian
`DataView.setFloat64` and `DataView.getFloat64`.

The WASM deterministic layout uses f64 size 8 and alignment 8. The C ABI uses
the target C compiler's `double` layout; on the release targets covered by
tests this is expected to be size 8 and alignment 8. CK / CalcKernel does not
promise bit-identical floating point results across all C, LLVM, WASM, and
JavaScript targets.

Semantic lock for f64:

- `f64` is the only floating point type; `f32` is not planned.
- C uses ordinary `double` operations.
- LLVM emits `double` operations without fast-math flags.
- WASM emits `f64` operations and exposes scalar f64 values to JavaScript as
  `Number`.
- Implicit int/float conversion is not supported.
- Exact explicit `i32_to_f64` and `u32_to_f64` casts are supported and do not
  change any exported ABI shape.
- `i64_to_f64`, `u64_to_f64`, and f64-to-int casts are not supported.
- NaN, infinity, and `-0.0` follow ordinary backend IEEE-like behavior.
- NaN payloads and cross-backend bit identity are not part of the ABI contract.
- Finite cross-backend tests must use tolerance; NaN, infinity, signed zero,
  and bool comparison results must be classified explicitly.

## Type Mapping

| CalcKernel type | C ABI type |
| --- | --- |
| `i32` | `int32_t` |
| `i64` | `int64_t` |
| `u32` | `uint32_t` |
| `u64` | `uint64_t` |
| `f64` | `double` |
| `bool` | `bool` |
| `ptr<T>` | `T*` |
| `struct` | `typedef struct` |

## Explicit Cast Lowering

Phase 20 supports only exact explicit `i32`/`u32` to `f64` compiler builtins:

| CK builtin | C lowering | WASM lowering | LLVM lowering |
| --- | --- | --- | --- |
| `i32_to_f64(x)` | `(double)x` | `f64.convert_i32_s` | `sitofp i32 ... to double` |
| `u32_to_f64(x)` | `(double)x` | `f64.convert_i32_u` | `uitofp i32 ... to double` |

These casts are not runtime calls and do not imply assignment conversion or
mixed arithmetic conversion. `let x: f64 = 1` and `i32_value + 1.0` remain type
errors. Division by zero after a cast follows ordinary strict f64 behavior and
does not produce checked integer errors.

Example:

```ck
struct Item {
  price: i64;
  qty: i64;
}
```

emits:

```c
typedef struct Item {
  int64_t price;
  int64_t qty;
} Item;
```

## Header ABI

Generated headers use `#pragma once` and include the standard integer and bool
headers. Checked-mode headers also include `stddef.h` for `NULL`:

```c
#pragma once

#include <stdint.h>
#include <stdbool.h>
/* checked mode also emits: #include <stddef.h> */
```

Exported functions use the `CK_API` macro:

```c
#if defined(_WIN32) || defined(__CYGWIN__)
  #ifdef CK_BUILD_DLL
    #define CK_API __declspec(dllexport)
  #else
    #define CK_API __declspec(dllimport)
  #endif
#else
  #define CK_API __attribute__((visibility("default")))
#endif
```

When compiling generated C into a dynamic library, define `CK_BUILD_DLL`. This
marks exported functions as library definitions on Windows and keeps one build
contract across platforms.

Generated headers are also safe to include from C++ translation units:

```c
#ifdef __cplusplus
extern "C" {
#endif

/* typedef structs and CK_API function declarations */

#ifdef __cplusplus
}
#endif
```

The `extern "C"` block prevents C++ name mangling for exported functions.

An `export fn` such as:

```ck
export fn calc(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32
```

emits:

```c
CK_API int32_t calc(Item* items, int32_t len, int64_t* out);
```

An `f64` scalar maps to C `double`, and `ptr<f64>` maps to `double*`:

```ck
export fn scale(value: f64, out: ptr<f64>) -> f64
```

emits a header signature shaped as:

```c
CK_API double scale(double value, double* out);
```

Non-exported `fn` declarations are not emitted into the header. They are emitted
as `static` functions in the generated `.c` file and are not part of the public
ABI.

## Dynamic Libraries

The CLI `build` command emits `.c` and `.h` files, then invokes clang with
strict flags and `-DCK_BUILD_DLL`.

Platform output names:

| Platform | Extension | Example |
| --- | --- | --- |
| macOS | `.dylib` | `build/libpricing.dylib` |
| Linux | `.so` | `build/libpricing.so` |
| Windows | `.dll` | `build/pricing.dll` |

macOS:

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DCK_BUILD_DLL \
  -shared -fPIC pricing.c \
  -o libpricing.dylib
```

Linux:

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DCK_BUILD_DLL \
  -shared -fPIC pricing.c \
  -o libpricing.so
```

Windows:

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DCK_BUILD_DLL \
  -shared pricing.c \
  -o pricing.dll
```

## Struct Layout

CalcKernel preserves struct field order exactly as written in the `.ck` source.
V0 uses the target C compiler's natural struct alignment rules. It does not emit
packed structs, `#pragma pack`, or custom alignment attributes.

FFI bindings must define host-side structs with the same field order, field
types, and C layout as the generated header. Do not assume packed layout.
`f64` fields use the target C compiler's `double` size and alignment; on the
release targets covered by tests, this is expected to be size 8 and alignment 8.
The C ABI intentionally follows the C compiler's layout rather than promising a
platform-independent binary layout.

For `examples/pricing.ck`, the generated `Item` layout is:

| Field | C type | Offset |
| --- | --- | --- |
| `price` | `int64_t` | 0 |
| `qty` | `int64_t` | 8 |
| `discount` | `int64_t` | 16 |
| `tax_rate_ppm` | `int64_t` | 24 |

`sizeof(Item)` is expected to be 32 for that example under the V0 C ABI tests.

## Buffer Ownership

The caller is responsible for all memory crossing the ABI:

- allocate input buffers
- allocate output buffers
- keep buffers alive for the full call
- pass valid pointers
- pass a valid `len`
- ensure output buffers are large enough for the function's writes

An CalcKernel-generated function only reads and writes the memory it is given.
It does not allocate replacement buffers and does not retain pointers after the
call returns.

This is why examples use an output pointer such as `out: ptr<i64>` rather than
returning an allocated array.

## Safety Limitations

V0 is deliberately close to C:

- arithmetic is unchecked by default
- pointer indexing has no bounds check
- invalid pointers are undefined behavior
- invalid lengths are undefined behavior
- undersized output buffers are undefined behavior
- in unchecked mode, division by zero follows generated C behavior
- in checked mode, arithmetic overflow and division by zero return `CK_Status`
  errors, but memory safety is still the caller's responsibility
- `f64` maps to C `double` and the C backend supports scalar f64 arithmetic,
  comparisons, `ptr<f64>`, and struct fields; checked mode does not add floating
  overflow or floating division-by-zero errors

Callers and DSL authors must choose sufficiently wide integer types, validate
lengths, and pass correct buffers.

## Checked Arithmetic ABI

Unchecked arithmetic remains the default ABI. With `--overflow unchecked`,
exported functions keep their original C signatures, non-exported functions
keep their original `static` signatures, and expressions are emitted directly
as C.

Phase 10 introduces an optional checked arithmetic mode:

```sh
ckc emit-c input.ck --out build/input.c --header build/input.h --overflow checked
ckc build input.ck --out build/libinput --overflow checked
```

Checked mode changes the generated C ABI. Exported functions return
`CK_Status`, and the source-level return value is written through a final
generated output pointer named `ck_return`:

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
```

For a source function:

```ck
export fn add_i64(a: i64, b: i64) -> i64
```

unchecked mode emits:

```c
CK_API int64_t add_i64(int64_t a, int64_t b);
```

checked mode emits:

```c
CK_API CK_Status add_i64(int64_t a, int64_t b, int64_t* ck_return);
```

The signature rule is:

- C return type becomes `CK_Status`
- original parameters are preserved in order
- a final `T* ck_return` parameter is appended, where `T` is the mapped C type
  of the original CalcKernel return type
- if `ck_return == NULL`, generated checked code returns
  `CK_ERR_NULL_POINTER`
- on success, generated checked code writes the original return value into
  `*ck_return` and returns `CK_OK`

Example:

```ck
export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32
```

checked mode emits:

```c
CK_API CK_Status calc_items(
  Item* items,
  int32_t len,
  int64_t* out,
  int32_t* ck_return
);
```

Non-exported CalcKernel functions also use checked lowering in checked mode, but
they remain private to the generated `.c` file:

```c
static CK_Status helper(int64_t a, int64_t* ck_return);
```

Callers never call non-exported helpers directly.

Checked mode reports integer arithmetic overflow, integer division by zero,
signed integer division or modulo overflow such as `INT64_MIN / -1`, and integer
unary minus overflow such as `-INT64_MIN`. `f64` arithmetic uses ordinary C
`double` behavior in checked mode: f64 division by zero and f64 overflow do not
return `CK_ERR_DIV_BY_ZERO` or `CK_ERR_OVERFLOW`. Checked mode does not add
pointer bounds checks or automatic checks for user-provided `ptr<T>` parameters.

Because checked mode changes signatures, unchecked and checked dynamic
libraries should be treated as distinct ABI artifacts.

See [Checked Arithmetic Design](CHECKED_ARITHMETIC.md) for the full Phase 10
design.

## Language Interop Notes

### C

Include the generated header and link against the generated object file or
dynamic library. Use the exact C types shown in the header.

### C++

Generated headers include an `extern "C"` guard, so C++ code can include them
directly without exported function name mangling.

### Python ctypes

Mirror generated structs with `ctypes.Structure`. Map:

- `i32` / `int32_t` to `ctypes.c_int32`
- `i64` / `int64_t` to `ctypes.c_int64`
- `u32` / `uint32_t` to `ctypes.c_uint32`
- `u64` / `uint64_t` to `ctypes.c_uint64`
- `ptr<T>` to `ctypes.POINTER(T)` or a caller-owned ctypes array

For checked functions, set `restype = ctypes.c_int32` for `CK_Status`, append a
pointer to the original return value to `argtypes`, pass it with
`ctypes.byref(...)`, and inspect the returned status before reading the value.

See `examples/python-ctypes-call`.

### Node.js

JavaScript `number` cannot exactly represent every `i64` or `u64` value. Prefer
`BigInt` or typed/native buffers for 64-bit integer values. The Koffi example
uses `BigInt` for `int64_t` fields and `BigInt64Array` for the `ptr<i64>` output
buffer.

For checked functions, bind the C return as an `int32` status and pass an extra
pointer argument for the original CalcKernel return value. Check `CK_OK` before
reading output buffers or the generated return pointer.

See `examples/node-ffi-call`.

### Rust

Use `#[repr(C)]` for mirror structs and C-compatible integer types from
`std::os::raw` or fixed-width Rust primitives such as `i32`, `i64`, `u32`, and
`u64`. Pointers should be represented as raw pointers such as `*const T` or
`*mut T` according to how the generated function reads or writes them.

### Go cgo

Use cgo to include the generated header and link the generated library. Mirror
struct layout through C types where possible, or use cgo-generated C struct
types directly to avoid layout drift.

### C# P/Invoke

Use `[DllImport]` for exported functions. Mirror structs with
`[StructLayout(LayoutKind.Sequential)]` and fields with matching fixed-width
integer types such as `int`, `long`, `uint`, and `ulong` where they correspond
to the generated C ABI types.

## Recommended Call Pattern

Batch work into large calls:

```c
calc_items(items, len, out);
```

Do not cross the host/native boundary once per item unless you have measured
that call overhead is acceptable. For Python, Node.js, C#, and other FFI users,
per-call overhead can dominate the kernel's compute time. Prefer passing arrays
and output buffers so the generated C code performs the loop inside one native
call.
