# IntKernel / TK LLVM Backend Design

[简体中文](zh-CN/LLVM_BACKEND.md)

This document defines the Phase 13 v1 LLVM backend design. It is a design
document only until the backend is implemented.

## Goal

IntKernel / TK adds an LLVM backend after MIR:

```text
.tk source
  -> lexer
  -> parser
  -> AST
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> LLVM IR text backend
  -> .ll
  -> clang / llc
  -> object file or native library
```

The backend must consume validated MIR. It must not generate LLVM IR directly
from AST.

Future native-library pipeline:

```text
.ll
  -> clang
  -> .so / .dylib / .dll
```

## Phase 13 v1 Scope

Supported:

- `i32`
- `i64`
- `u32`
- `u64`
- `bool`
- `ptr<T>`
- `struct`
- exported functions
- internal non-exported functions
- scalar arithmetic
- comparisons
- `if` / `else`
- `while`
- function calls
- ptr/index/field load and store
- unchecked arithmetic

Not supported:

- checked LLVM backend
- optimizer pass pipeline
- LLVM C++ API bindings
- LLVM bitcode writer
- JIT
- debug info
- DWARF
- LTO
- bounds check
- `slice<T>`
- runtime
- allocator
- strings
- IO
- module system

## LLVM IR Generation Strategy

Phase 13 v1 emits textual LLVM IR:

```text
MIR -> .ll
```

It does not embed LLVM libraries and does not call the LLVM C++ API.

Reasons:

- TypeScript can generate stable text without native LLVM bindings.
- `.ll` output is readable and reviewable.
- snapshots can lock generated IR format and ABI shape.
- `clang` or `llc` can validate syntax and compile generated IR.

## SSA Strategy

LLVM IR is SSA, but MIR v1 is not SSA. Phase 13 v1 uses
alloca/load/store lowering:

- each parameter, local, and temporary gets an `alloca` in the entry block
- function parameters are stored into their corresponding allocas at function
  entry
- each MIR instruction loads operands, computes a result, and stores the result
  into the target alloca
- later clang/LLVM optimization can promote memory to registers with mem2reg

This is not optimal IR. It is deliberately simple, correct, stable, and easy to
debug. A future phase can add direct SSA lowering or a MIR-to-SSA transform.

## Type Mapping

| IntKernel / TK type | LLVM IR type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `bool` internal | `i1` |
| `ptr<T>` | `ptr` |
| `struct` | named LLVM struct type |

Phase 13 v1 uses LLVM opaque pointers (`ptr`).

Signedness is not part of the integer type. Signed and unsigned differences are
encoded by instruction choice for division, remainder, and comparison.

`bool` ABI is intentionally conservative in v1. Internal boolean values are
`i1`. Cross-language bool ABI is not the focus of Phase 13 v1; scalar
conditions and boolean results should be covered first, and exported bool ABI
should be documented carefully before being treated as stable.

## Struct Types

Structs lower to named LLVM struct types:

```llvm
%struct.Item = type { i64, i64, i64, i64 }
```

Field order follows the source declaration order. Layout is ultimately
interpreted by the LLVM target data layout during native compilation, so Phase
13 tests must verify important ABI expectations with clang on supported hosts.

## Arithmetic Mapping

Unchecked arithmetic:

| MIR op | Signed type | Unsigned type |
| --- | --- | --- |
| `+` | `add` | `add` |
| `-` | `sub` | `sub` |
| `*` | `mul` | `mul` |
| `/` | `sdiv` | `udiv` |
| `%` | `srem` | `urem` |

Phase 13 v1 must not add checked arithmetic guards. If checked mode is
requested, the backend must report an unsupported-mode error.

## Comparison Mapping

Equality:

- `==` -> `icmp eq`
- `!=` -> `icmp ne`

Signed ordering:

- `<` -> `icmp slt`
- `<=` -> `icmp sle`
- `>` -> `icmp sgt`
- `>=` -> `icmp sge`

Unsigned ordering:

- `<` -> `icmp ult`
- `<=` -> `icmp ule`
- `>` -> `icmp ugt`
- `>=` -> `icmp uge`

Comparison results are `i1`.

## Control Flow

MIR blocks map directly to LLVM basic blocks.

MIR terminators lower as:

- `return value` -> `ret <type> <value>`
- `jump label` -> `br label %label`
- `branch cond then else` -> `br i1 %cond, label %then, label %else`

Short-circuit behavior is already represented as MIR control flow, so the LLVM
backend must preserve block structure instead of re-evaluating logical RHS
expressions.

## Function Calls

MIR call instructions lower to LLVM `call` instructions.

Exported function example:

```llvm
define i32 @calc_items(ptr %items, i32 %len, ptr %out) {
  ...
}
```

Internal non-exported function example:

```llvm
define internal i64 @add_i64(i64 %a, i64 %b) {
  ...
}
```

Function definition order should be stable. Forward references are allowed by
LLVM IR, but stable module order is better for snapshots.

## Struct and Pointer Access

For:

```ik
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}
```

LLVM struct type:

```llvm
%struct.Item = type { i64, i64, i64, i64 }
```

`items[i].price` lowers to GEP + load:

```llvm
%ptr_item = getelementptr %struct.Item, ptr %items, i64 %idx
%ptr_price = getelementptr %struct.Item, ptr %ptr_item, i32 0, i32 0
%price = load i64, ptr %ptr_price
```

`out[i] = value` lowers to GEP + store:

```llvm
%ptr_out_i = getelementptr i64, ptr %out, i64 %idx
store i64 %value, ptr %ptr_out_i
```

Index expressions are evaluated by MIR before address lowering. Phase 13 v1
does not add bounds checks.

## Target Triple

`emit-llvm` may support an optional target triple:

```sh
tkc emit-llvm examples/pricing.tk --out build/pricing.ll --target x86_64-apple-darwin
```

Common triples:

- `x86_64-apple-darwin`
- `arm64-apple-darwin` or `aarch64-apple-darwin`
- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-pc-windows-msvc`

If no target is provided, Phase 13 v1 may omit the `target triple` line or use
native target detection. Omitting it is acceptable for the initial textual IR
backend.

## CLI Design

Proposed commands:

```sh
tkc emit-llvm examples/pricing.tk --out build/pricing.ll
tkc emit-llvm examples/pricing.tk --out build/pricing.ll --target x86_64-apple-darwin

tkc build-llvm examples/pricing.tk --out build/libpricing
```

If the current package still exposes `ikc`, the same backend may be introduced
there first:

```sh
ikc emit-llvm examples/pricing.ik --out build/pricing.ll
ikc build-llvm examples/pricing.ik --out build/libpricing
```

`emit-llvm` must be pure text generation and must not require clang or LLVM
tools. `build-llvm` may invoke clang.

## build-llvm

`build-llvm` can compile generated `.ll` through clang.

macOS:

```sh
clang -O3 -shared -fPIC build/pricing.ll -o build/libpricing.dylib
```

Linux:

```sh
clang -O3 -shared -fPIC build/pricing.ll -o build/libpricing.so
```

Windows:

```sh
clang -O3 -shared build/pricing.ll -o build/pricing.dll
```

If clang is not available, `build-llvm` should print a friendly error.
`emit-llvm` must remain available without clang.

## Checked Mode

Phase 13 v1 does not support checked LLVM code generation.

If a user runs:

```sh
tkc emit-llvm input.tk --overflow checked
```

the compiler must report:

```text
LLVM backend does not support --overflow checked yet.
```

The backend must not silently generate unchecked LLVM IR when checked mode is
requested.

## Testing Strategy

Required tests:

- LLVM IR golden snapshots
- LLVM syntax smoke test when clang is available
- clang compile `.ll` to executable or native library when clang is available
- scalar e2e
- control-flow e2e
- function-call e2e
- ptr/index/field/store e2e
- pricing e2e
- checked-mode unsupported diagnostic tests
- C backend regression tests
- WASM backend regression tests

Generated LLVM IR must be stable:

- no absolute paths
- no timestamps
- no random IDs
- normalized `\n` newlines

## Risks

- bool ABI and whether exported bool results should be `i1`, `i8`, or `i32`
- struct layout and target data layout differences
- opaque pointer syntax compatibility with host clang versions
- Windows linking and symbol export behavior
- LLVM tool availability in local and CI environments
- alloca-heavy IR performance is not the final shape
- matching C backend ABI behavior for host-language integrations

## Future Work

- direct SSA lowering
- optional optimizer pipeline
- checked LLVM arithmetic lowering
- target-specific data layout emission
- debug info
- bitcode emission
- object/native-library build hardening
