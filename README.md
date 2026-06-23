# IntKernel

IntKernel is a small integer-computation DSL compiler. It is not a general
purpose programming language. V0 compiles `.ik` source into readable C source
and header files, which can then be compiled into a dynamic library for host
languages such as Node.js, Python, Java, Rust, Go, and C#.

The project is intentionally narrow: pure integer kernels, caller-owned memory,
no runtime, and no dynamic allocation.

V0.1 has ABI hardening for C/C++ headers, dynamic-library symbol exports,
struct layout verification, Python `ctypes` integration, a Node.js FFI example,
and a small benchmark harness.

## Quick Start

```sh
pnpm install
pnpm test
pnpm build
```

In a source checkout, run the built CLI through the local pnpm script:

```sh
pnpm ikc --help
pnpm ikc check examples/pricing.ik
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h
pnpm ikc build examples/pricing.ik --out build/libpricing
pnpm ikc build examples/pricing.ik --out build/libpricing --overflow unchecked
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
```

When installed as a package, the `bin` entrypoint is `ikc`.

## Example `.ik`

```ik
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}

export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
  let i: i32 = 0;

  while i < len {
    let subtotal: i64 = items[i].price * items[i].qty;
    let after_discount: i64 = subtotal - items[i].discount;
    let tax: i64 = after_discount * items[i].tax_rate_ppm / 1000000;
    out[i] = after_discount + tax;
    i = i + 1;
  }

  return 0;
}
```

## Generate C

Unchecked output is the default:

```sh
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h
```

The explicit unchecked form is equivalent:

```sh
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h --overflow unchecked
```

Checked output uses the checked arithmetic ABI:

```sh
pnpm ikc emit-c examples/pricing.ik --out build/pricing.checked.c --header build/pricing.checked.h --overflow checked
```

The generated header includes `stdint.h`, `stdbool.h`, struct typedefs, and
exported function declarations. Headers also include `IK_API` for dynamic
library exports and an `extern "C"` guard for C++ consumers. The generated
source includes the header and function implementations.

## Build a Dynamic Library

Unchecked mode is the default:

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing
```

This is equivalent to:

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing --overflow unchecked
```

Checked arithmetic mode changes the generated C ABI to return `IK_Status` and
write the original return value through the final `ik_return` pointer:

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
```

`IK_Status` is an `int32_t` result code:

- `IK_OK`: computation succeeded
- `IK_ERR_OVERFLOW`: checked arithmetic overflow
- `IK_ERR_DIV_BY_ZERO`: checked division or modulo by zero
- `IK_ERR_NULL_POINTER`: generated checked `ik_return` pointer was `NULL`

Use checked mode for money, tax, discount, and rules kernels where arithmetic
failure must be reported explicitly. Use unchecked mode for hot paths where
inputs have already been validated and maximum throughput matters more than
per-operation checks.

The build command emits C/header files and invokes clang with strict flags:

```text
-std=c11 -O3 -Wall -Wextra -Werror
```

The output extension is platform-specific:

- Linux: `.so`
- macOS: `.dylib`
- Windows: `.dll`

## Developer MIR Debugging

Developers can inspect the typed MIR used by the default C backend:

```sh
pnpm ikc emit-mir examples/pricing.ik
pnpm ikc emit-mir examples/pricing.ik --out build/pricing.mir
```

MIR is an internal compiler IR: it is typed, basic-block based, and designed for
backend implementation and debugging. It is not a user-facing source language.
Normal users should continue to use `check`, `emit-c`, and `build`.

## Python ctypes Example

The repository includes a no-dependency Python example that calls the generated
pricing dynamic library through `ctypes`.

On macOS/Linux:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/libpricing
python3 examples/python-ctypes-call/call_pricing.py
```

On Windows, generate `pricing.dll` and run the same script with Python:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing.dll
py examples\python-ctypes-call\call_pricing.py
```

Checked ABI example:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
python3 examples/python-ctypes-call/call_pricing_checked.py
```

On Windows, generate `pricing_checked.dll` explicitly:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing_checked.dll --overflow checked
py examples\python-ctypes-call\call_pricing_checked.py
```

See [examples/python-ctypes-call](examples/python-ctypes-call/README.md) for
the `ctypes` struct, pointer, and checked `IK_Status` mapping.

## Node.js FFI Example

The repository also includes an isolated Node.js FFI example. Its native FFI
dependency lives only under the example directory, not in the root package.

On macOS/Linux:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/libpricing
cd examples/node-ffi-call
pnpm install
pnpm start
```

On Windows, generate `pricing.dll` first:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing.dll
cd examples\node-ffi-call
pnpm install
pnpm start
```

Checked ABI example:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
cd examples/node-ffi-call
pnpm install
pnpm start:checked
```

On Windows, generate `pricing_checked.dll` explicitly:

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing_checked.dll --overflow checked
cd examples\node-ffi-call
pnpm install
pnpm start:checked
```

See [examples/node-ffi-call](examples/node-ffi-call/README.md) for the Koffi
struct, pointer, `BigInt`, and checked `IK_Status` mapping.

## Benchmarks

The [bench](bench/README.md) directory contains a small pricing benchmark for a
pure JavaScript baseline and the generated C implementation.

```sh
pnpm build
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h
node bench/pricing_baseline.js
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL build/pricing.c bench/pricing_c_harness.c -I build -o build/pricing_c_bench
./build/pricing_c_bench
```

The benchmark is a rough local reference. For host-language integration, batch
work into larger native calls rather than calling one item at a time.
The benchmark README also includes unchecked vs checked C benchmark commands.

## Current V0 Limits

V0 supports only:

- `i32`, `i64`, `u32`, `u64`, `bool`
- `ptr<T>`
- `struct`
- `fn` and `export fn`
- `let`, assignment, `return`, `if` / `else`, `while`
- integer arithmetic, comparison, logical operators
- pointer indexing and struct field access

V0 does not support strings, IO, heap allocation, GC, exceptions, async,
classes, closures, modules, runtime libraries, LLVM, WASM, or JIT compilation.

V0 does not perform bounds checks. By default arithmetic is unchecked; optional
`--overflow checked` code generation checks integer overflow and division by
zero but still does not check pointer validity or buffer lengths. Callers own
all input and output buffers and must pass valid pointers and lengths.

## Documentation

- [Language Specification](docs/LANGUAGE_SPEC.md)
- [Compiler Architecture](docs/COMPILER_ARCHITECTURE.md)
- [MIR](docs/MIR.md)
- [Checked Arithmetic](docs/CHECKED_ARITHMETIC.md)
- [C ABI](docs/ABI.md)
- [Roadmap](docs/ROADMAP.md)
- [Release Checklist](docs/RELEASE_CHECKLIST.md)
