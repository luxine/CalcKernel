# IntKernel

[简体中文](README.zh-CN.md)

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

## WASM Backend

Phase 12 adds a WASM backend that lowers validated MIR to WAT and then compiles
WAT to WASM with the bundled `wabt` npm package:

```sh
pnpm ikc emit-wat examples/scalar.ik --out build/scalar.wat
pnpm ikc emit-wasm examples/scalar.ik --out build/scalar.wasm
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
```

The Phase 12 v1 ABI targets `wasm32`, exports linear memory, maps `ptr<T>` to
`i32` memory offsets, uses `BigInt` for JavaScript `i64` / `u64` interop, and
keeps arithmetic unchecked. The current backend covers scalar operations,
control flow, internal function calls, short-circuit logic, and core
ptr/index/field load/store patterns such as `pricing.ik`.

The WASM backend currently supports unchecked mode only. `emit-wat --overflow
checked` and `emit-wasm --overflow checked` fail with a clear error; use
`emit-c` or `build` when checked arithmetic is required. Checked WASM code
generation and bounds checks are not implemented yet.

See [WASM ABI](docs/WASM_ABI.md) for the ABI, struct layout, memory model, WABT
assembly step, and Node.js interop rules.

## LLVM Backend

Phase 13 adds a MIR-to-LLVM backend that emits textual LLVM IR (`.ll`) and can
optionally invoke clang to build a native dynamic library:

```text
.ik / .ik source -> CheckedProgram -> MIR -> LLVM IR text
```

```sh
pnpm ikc emit-llvm examples/pricing.ik --out build/pricing.ll
pnpm ikc build-llvm examples/pricing.ik --out build/libpricing
pnpm ikc build-llvm examples/pricing.ik --kind object --out build/pricing.o
pnpm ikc build-llvm examples/pricing.ik --out build/libpricing --target x86_64-unknown-linux-gnu
```

The v1 backend is unchecked-only: `emit-llvm --overflow checked` and
`build-llvm --overflow checked` fail instead of silently producing unchecked
LLVM IR. Use the C backend when checked arithmetic is required. LLVM v1 does
not use LLVM C++ API bindings, JIT, an optimizer pipeline, debug info, runtime
support, allocator, bounds checks, or `slice<T>`. `build-llvm` requires clang;
`emit-llvm` works without clang. Object output is available for custom link
flows; static library output is not implemented yet. See
[LLVM Backend](docs/LLVM_BACKEND.md) for the backend details.

## Node.js WASM Example

The repository includes a no-dependency Node.js WASM example that calls
`calc_items` through the built-in WebAssembly API. It does not need a native
`.so`, `.dylib`, or `.dll`.

```sh
pnpm build
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
node examples/node-wasm-call/index.mjs
```

See [examples/node-wasm-call](examples/node-wasm-call/README.md) for the
`DataView` memory writes, `Item` layout, pointer offsets, output buffer, and
`BigInt` mapping.

## Browser WASM Example

The repository also includes a plain browser WASM example with no framework or
bundler. Generate `pricing.wasm` into the browser example directory and serve it
over HTTP:

```sh
pnpm build
pnpm ikc emit-wasm examples/pricing.ik --out examples/browser-wasm-call/pricing.wasm
cd examples/browser-wasm-call
python3 -m http.server 8000
```

Open `http://localhost:8000/index.html` and click **Run pricing wasm**.

Browsers generally cannot fetch WASM from `file://`, so use a local HTTP server.
See [examples/browser-wasm-call](examples/browser-wasm-call/README.md) for the
full browser memory and `DataView` notes.

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

The [bench](bench/README.md) directory contains the local pricing performance
suite. It compares generated C, checked C, LLVM, WASM, and JavaScript baselines
with checksum validation.

```sh
pnpm build
node bench/perf/run.mjs --quick
node bench/perf/run.mjs --full --save-baseline
node bench/perf/run.mjs --full --compare --threshold 10
```

The benchmark suite is a rough local reference, not a stable cross-machine
score. For host-language integration, batch work into larger native calls rather
than calling one item at a time. See [Performance](docs/PERFORMANCE.md) and
[Optimization](docs/OPTIMIZATION.md) for the current Phase 14 pipeline, latest
local full-run summary, regression baseline workflow, and backend bottlenecks.

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
classes, closures, modules, runtime libraries, or JIT compilation. The WASM
and LLVM backends currently support unchecked arithmetic only.

V0 does not perform bounds checks. By default arithmetic is unchecked; optional
`--overflow checked` C code generation checks integer overflow and division by
zero but still does not check pointer validity or buffer lengths. The WASM
backend rejects `--overflow checked` in Phase 12. Callers own all input and
output buffers and must pass valid pointers and lengths.

## Documentation

English is the default documentation language. Chinese translations are kept in
parallel for every project document.

- [Language Specification](docs/LANGUAGE_SPEC.md)
- [Compiler Architecture](docs/COMPILER_ARCHITECTURE.md)
- [MIR](docs/MIR.md)
- [Checked Arithmetic](docs/CHECKED_ARITHMETIC.md)
- [C ABI](docs/ABI.md)
- [WASM ABI](docs/WASM_ABI.md)
- [LLVM Backend](docs/LLVM_BACKEND.md)
- [Optimization](docs/OPTIMIZATION.md)
- [Performance](docs/PERFORMANCE.md)
- [Naming Conventions](docs/NAMING_CONVENTIONS.md)
- [Roadmap](docs/ROADMAP.md)
- [Release Checklist](docs/RELEASE_CHECKLIST.md)

Chinese:

- [语言规格](docs/zh-CN/LANGUAGE_SPEC.md)
- [编译器架构](docs/zh-CN/COMPILER_ARCHITECTURE.md)
- [MIR](docs/zh-CN/MIR.md)
- [Checked Arithmetic](docs/zh-CN/CHECKED_ARITHMETIC.md)
- [C ABI](docs/zh-CN/ABI.md)
- [WASM ABI](docs/zh-CN/WASM_ABI.md)
- [LLVM Backend](docs/zh-CN/LLVM_BACKEND.md)
- [优化](docs/zh-CN/OPTIMIZATION.md)
- [性能](docs/zh-CN/PERFORMANCE.md)
- [命名规范](docs/zh-CN/NAMING_CONVENTIONS.md)
- [路线图](docs/zh-CN/ROADMAP.md)
- [发布检查清单](docs/zh-CN/RELEASE_CHECKLIST.md)
