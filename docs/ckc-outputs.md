# ckc Outputs and Dependencies

[Simplified Chinese](zh-CN/ckc-outputs.md)

This document describes the artifacts produced by `native ckc` and the tools
needed to use them.

## Commands

| Command | Output | Generation dependency | Follow-up dependency |
| --- | --- | --- | --- |
| `ckc check input.ck` | none | `ckc` | none |
| `ckc emit-mir input.ck` | MIR text | `ckc` | none |
| `ckc emit-c input.ck -o input.c` | C source plus header | `ckc` | C compiler to build |
| `ckc emit-wat input.ck -o input.wat` | WAT text | `ckc` | none |
| `ckc emit-wasm input.ck -o input.wasm` | WASM binary | `ckc` | WebAssembly runtime |
| `ckc emit-llvm input.ck -o input.ll` | LLVM IR text | `ckc` | LLVM tools to build |
| `ckc build input.ck -o output` | dynamic library | `ckc` plus clang | host loader / FFI |
| `ckc build-llvm input.ck --kind object -o input.o` | object file | `ckc` plus clang | linker |

Generation dependencies and use-time dependencies are separate. For example,
`emit-c` only needs `ckc` to write C and header files, but compiling that C
requires a C compiler.

## Native CLI

```sh
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/scalar.ck
```

Installed release artifacts can be invoked as `ckc` directly.

## MIR

```sh
ckc emit-mir examples/scalar.ck -O3
ckc emit-mir examples/scalar.ck -O3 -o build/scalar.mir
```

MIR is a compiler debugging artifact. It is useful for lowering, optimizer, and
snapshot work. It is not executable.

## C

```sh
ckc emit-c examples/pricing.ck --out build/pricing.c --header build/pricing.h
clang -O3 -c build/pricing.c -o build/pricing.o
```

The C path is suitable for native integration, C ABI use, FFI, and embedding CK
kernels into services. Checked overflow mode is available through the C backend.

## WASM

```sh
ckc emit-wat examples/wasm_scalar.ck --out build/scalar.wat
ckc emit-wasm examples/wasm_scalar.ck --out build/scalar.wasm
```

`emit-wasm` writes a WebAssembly binary. The binary needs a WebAssembly runtime
to instantiate it. The generated module follows caller-owned memory rules: the
host chooses offsets, writes inputs into linear memory, calls exported
functions, and reads outputs back.

Recommended host patterns:

- use typed-array views for homogeneous buffers
- use `DataView` for mixed-width structs and byte-exact ABI checks
- recreate views after `memory.grow`
- keep input/output regions non-overlapping unless the kernel is designed for
  in-place updates

## LLVM

```sh
ckc emit-llvm examples/llvm_scalar.ck --out build/scalar.ll
ckc build-llvm examples/llvm_scalar.ck --kind object --out build/scalar.o
```

`emit-llvm` writes textual LLVM IR and does not require clang. `build-llvm`
invokes clang and produces either an object file or a dynamic library depending
on `--kind`.

## Unsupported Direct Outputs

The current native CLI does not directly provide:

- a standalone executable from `ckc emit-*`
- a dependency-free `.so` / `.dylib` / `.dll` generator
- a Python wheel
- a Java jar
- GPU, CUDA, or WebGPU shader output
- SIMD WASM output
- f32 kernels
- `ckc run input.ck`

Some native library flows exist through `ckc build` and `ckc build-llvm`, but
those commands still depend on the platform toolchain.

## Slice and bounds output matrix

All emitters lower valid `slice<T>` programs. `ckc emit-c` and `ckc build`
accept all combinations of `--overflow unchecked|checked` and `--bounds
unchecked|checked`; a checked mode selects the full five-code status ABI. `ckc
emit-wat`, `emit-wasm`, `emit-llvm`, and `build-llvm` accept `--bounds unchecked`
and reject checked bounds with stable backend-specific help. `check` and
`emit-mir` are source-semantic commands and do not consume backend bounds mode.

C headers expose typed descriptors for stored fields but flatten slice
parameters. WASM artifacts expose paired address/length parameters. LLVM IR
exposes flattened `ptr, i32` parameters and `{ ptr, i32 }` stored values.
