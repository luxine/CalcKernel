# CK / CalcKernel LLVM Backend

[简体中文](zh-CN/LLVM_BACKEND.md)

This document defines the Phase 13 v1 LLVM backend behavior, ABI assumptions,
external tool dependencies, limitations, and future work.

## Goal

CK / CalcKernel adds an LLVM backend after MIR:

```text
.ck source
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

Native-library pipeline:

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
- `f64` scalar arithmetic and comparisons
- `if` / `else`
- `while`
- function calls
- ptr/index/field load and store
- `ptr<f64>` and struct fields containing `f64`
- unchecked arithmetic

Not supported:

- checked LLVM backend
- LLVM-specific optimizer pass pipeline
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
debug.

Phase 14.14 adds a small SSA-like fast path for simple scalar straight-line
functions at `-O2` and `-O3`. The fast path is limited to one basic block with
no locals, no calls, no control flow, and no memory operations. More complex
functions, including `examples/pricing.ck`, continue to use stack lowering and
rely on clang `-O2`/`-O3` for promotion and native optimization. A future phase
can broaden direct SSA lowering or add a MIR-to-SSA transform.

## Type Mapping

| CK / CalcKernel type | LLVM IR type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `f64` | `double` |
| `bool` internal and exported scalar | `i1` |
| `ptr<T>` | `ptr` |
| `struct` | named LLVM struct type |

Phase 13 v1 uses LLVM opaque pointers (`ptr`).
Phase 16.8 supports `f64` as LLVM `double`, including scalar parameters and
returns, `ptr<f64>` memory access, struct fields as `double`, arithmetic,
unary negation, and comparisons. LLVM f64 codegen does not add fast-math flags
and does not promise bit-identical results across every backend.
Phase 20 supports exact explicit `i32_to_f64` and `u32_to_f64` casts using
`sitofp` and `uitofp`.

Signedness is not part of the integer type. Signed and unsigned differences are
encoded by instruction choice for division, remainder, and comparison.

### Bool ABI

Phase 13.7 keeps strategy A:

- internal bool values use `i1`
- conditions use `i1`
- bool locals and temporaries use `i1`
- exported bool parameters and return values use `i1`

On the current macOS clang target, compiling equivalent C such as:

```c
#include <stdbool.h>
bool less_i64(long long a, long long b) { return a < b; }
bool not_bool(bool a) { return !a; }
int choose_bool(bool a, int x, int y) { return a ? x : y; }
```

produces LLVM IR using `i1` for bool parameters and returns, with clang adding
ABI attributes such as `zeroext` on some signatures. CalcKernel's LLVM v1 emits
plain `i1`; the Phase 13.7 clang e2e verifies that a C harness using `bool` can
call exported LLVM functions for bool return, bool parameter, bool local, and
`if` on a bool parameter on the current target.

Cross-platform bool ABI remains a risk. If Windows or other targets require
attributes such as `zeroext` for stable interop, a later hardening phase should
add target-aware function parameter and return attributes rather than changing
the public language type.

## Struct Types

Structs lower to named LLVM struct types:

```llvm
%struct.Item = type { i64, i64, i64, i64 }
```

An `f64` field lowers to `double`:

```llvm
%struct.WithF64 = type { i32, double }
```

Field order follows the source declaration order. Layout is ultimately
interpreted by the LLVM target data layout during native compilation, so Phase
13 tests must verify important ABI expectations with clang on supported hosts.

## Arithmetic Mapping

Unchecked arithmetic:

| MIR op | Signed integer | Unsigned integer | `f64` |
| --- | --- | --- | --- |
| `+` | `add` | `add` | `fadd` |
| `-` | `sub` | `sub` | `fsub` |
| `*` | `mul` | `mul` | `fmul` |
| `/` | `sdiv` | `udiv` | `fdiv` |
| `%` | `srem` | `urem` | unsupported |

Unary `-f64` lowers to `fneg double`. The backend must not lower f64 negation
through integer zero subtraction.

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

F64 comparison:

- `==` -> `fcmp oeq`
- `!=` -> `fcmp une`
- `<` -> `fcmp olt`
- `<=` -> `fcmp ole`
- `>` -> `fcmp ogt`
- `>=` -> `fcmp oge`

Comparison results are `i1`.

F64 semantics are locked to strict LLVM IR:

- emit `fadd`, `fsub`, `fmul`, `fdiv`, `fneg`, and `fcmp` without fast-math
  flags
- emit `i32_to_f64` as `sitofp i32 ... to double`
- emit `u32_to_f64` as `uitofp i32 ... to double`
- do not use reassociation, reciprocal, no-NaN, no-infinity, signed-zero-ignore,
  or other fast-float assumptions
- NaN, infinity, and `-0.0` follow the compiled target's ordinary IEEE-like
  behavior
- do not promise stable NaN payloads or bit-identical behavior against C, WASM,
  or JavaScript hosts

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

```ck
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
extends `i32` indexes with `sext` and `u32` indexes with `zext` before using
them as LLVM `i64` GEP indexes. Phase 13 v1 does not add bounds checks and does
not check pointer nullness.

The Phase 13 memory e2e tests cover integer pointers and integer struct fields.
Bool scalar ABI is covered separately, but bool fields or bool pointers are not
treated as a cross-language stable LLVM memory ABI yet.

`ptr<f64>` uses opaque `ptr` at the function boundary and lowers indexed access
with `getelementptr double`. F64 loads and stores use `load double` and
`store double`. Struct fields declared as `f64` use `double` in the named LLVM
struct declaration, and field access uses the same struct GEP pattern as
integer fields.

## Target Triple

`emit-llvm` may support an optional target triple:

```sh
ckc emit-llvm examples/pricing.ck --out build/pricing.ll --target x86_64-apple-darwin
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

`build-llvm --target <triple>` writes the target triple into the generated
`.ll`. Phase 13.14 does not pass `--target` through to clang; target-aware clang
argument handling is a later hardening step.

## CLI

`emit-llvm` writes textual LLVM IR and does not require clang:

```sh
ckc emit-llvm examples/pricing.ck --out build/pricing.ll
ckc emit-llvm examples/pricing.ck --out build/pricing.ll --target x86_64-apple-darwin
```

If `--out` is omitted, `emit-llvm` writes the `.ll` text to stdout.

`build-llvm` emits LLVM IR, writes a temporary `.ll`, and invokes clang:

```sh
ckc build-llvm examples/pricing.ck --out build/libpricing
ckc build-llvm examples/pricing.ck --kind object --out build/pricing.o
```

If the current package still exposes `ckc`, the same backend may be introduced
there first:

```sh
ckc emit-llvm examples/pricing.ck --out build/pricing.ll
ckc build-llvm examples/pricing.ck --out build/libpricing
```

`emit-llvm` is pure text generation and does not require clang or LLVM tools.
`build-llvm` requires clang on `PATH`.

## build-llvm

`build-llvm` can compile generated `.ll` through clang.

Dynamic library output is the default:

The selected CK optimization level is passed to clang as `-O0`, `-O1`, `-O2`,
or `-O3`. The examples below show `--opt-level 3`.

macOS:

```sh
ckc build-llvm examples/pricing.ck --out build/libpricing --opt-level 3
clang -O3 -shared -fPIC build/libpricing.ll -o build/libpricing.dylib
```

Linux:

```sh
ckc build-llvm examples/pricing.ck --out build/libpricing --opt-level 3
clang -O3 -shared -fPIC build/libpricing.ll -o build/libpricing.so
```

Windows:

```sh
ckc build-llvm examples/pricing.ck --out build/pricing --opt-level 3
clang -O3 -shared build/pricing.ll -o build/pricing.dll
```

If the `--out` value has no dynamic-library extension, `build-llvm` adds the
platform extension: `.dylib` on macOS, `.so` on Linux, and `.dll` on Windows. If
the user passes a complete filename ending in `.so`, `.dylib`, or `.dll`, the
filename is respected.

Object output is available for user-managed linking:

```sh
ckc build-llvm examples/pricing.ck --kind object --out build/pricing.o --opt-level 3
clang -O3 -c build/pricing.ll -o build/pricing.o
```

On macOS and Linux, object output conventionally uses `.o`. On Windows,
`build-llvm --kind object --out build/pricing` uses `.obj`; if the user passes
an explicit `.o` filename, clang is allowed to produce that file. Static library
output is not implemented in Phase 13.15; callers can use their own linker,
`ar`, or `llvm-ar` outside CalcKernel if needed.

If clang is not available, `build-llvm` prints a friendly error and recommends
using `emit-llvm` to generate LLVM IR without clang. `emit-llvm` must remain
available without clang.

## Checked Mode

Phase 13 v1 does not support checked LLVM code generation.

The LLVM backend rejects checked mode for both LLVM entry points:

```sh
ckc emit-llvm input.ck --overflow checked
ckc build-llvm input.ck --overflow checked
```

the compiler must report:

```text
LLVM backend does not support --overflow checked yet.
Use --overflow unchecked, or use the C backend for checked arithmetic.
```

The backend must not silently generate unchecked LLVM IR when checked mode is
requested. Use the C backend (`emit-c` or `build`) when checked arithmetic is
required.

## Testing Strategy

Required tests:

- `emit-llvm` CLI tests
- `build-llvm` clang command tests
- LLVM IR golden snapshots
- LLVM syntax smoke test when clang is available
- clang compile `.ll` to executable or native library when clang is available
- scalar e2e
- control-flow e2e
- function-call e2e
- ptr/index/field/store e2e
- f64 scalar, pointer, and struct-field e2e
- no-fast-math regression for f64 LLVM IR
- pricing e2e
- checked-mode unsupported diagnostic tests
- object output tests
- C backend regression tests
- WASM backend regression tests
- C/WASM/LLVM backend behavior comparison tests

Generated LLVM IR must be stable:

- no absolute paths
- no timestamps
- no random IDs
- normalized `\n` newlines

## Risks

- bool ABI attributes such as `zeroext` may need target-aware hardening
- struct layout and target data layout differences
- opaque pointer syntax compatibility with host clang versions
- Windows linking and symbol export behavior
- LLVM tool availability in local and CI environments
- alloca-heavy IR performance is not the final shape
- matching C backend ABI behavior for host-language integrations
- cross-backend f64 precision and NaN behavior may differ in target-specific
  edge cases

## Current Limitations

- LLVM backend supports unchecked arithmetic only.
- `emit-llvm --overflow checked` and `build-llvm --overflow checked` fail with a
  documented unsupported-mode diagnostic.
- general control-flow and memory functions still use alloca/load/store
  lowering.
- only simple scalar straight-line functions use SSA-like lowering at `-O2` and
  `-O3`.
- f64 codegen is strict by default and does not emit fast-math flags.
- f64 `%`, f32, implicit int/float conversion, `i64_to_f64`, `u64_to_f64`,
  f64-to-int casts, float checked overflow, SIMD, and JIT remain unsupported.
- no LLVM-specific optimizer pass pipeline is run by CalcKernel; backend input
  still flows through the shared MIR pass manager.
- `build-llvm` depends on external clang; CalcKernel does not bundle clang,
  `llc`, LLVM libraries, or a custom linker.
- No static library output is built by CalcKernel yet.
- No debug info, DWARF, LTO, or bitcode writer.
- No runtime, allocator, JIT, strings, IO, modules, bounds checks, or
  `slice<T>`.
- Raw pointer validity and buffer sizes remain caller responsibilities.
- Cross-platform bool ABI attributes and target-specific data layout still need
  additional hardening before broad FFI guarantees.

## Future Work

- checked LLVM arithmetic lowering
- broader direct SSA LLVM lowering
- optional optimizer pass pipeline
- target-specific data layout emission
- object/static library output improvements
- target-aware clang argument handling
- debug info and DWARF
- bitcode emission
- JIT, if a future product use case justifies it
- `slice<T>` / bounds-check support after the language has length-carrying
  pointer types
