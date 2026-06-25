# IntKernel Roadmap

[简体中文](zh-CN/ROADMAP.md)

This roadmap tracks likely work after V0. It is not a promise that every item
will ship in this order.

## V0 Stable

- Keep the language intentionally small.
- Stabilize lexer, parser, type checker, diagnostics, MIR, C backend, WASM
  backend, LLVM backend, CLI, and tests.
- Maintain generated C/header golden snapshots.
- Keep strict clang e2e coverage where clang is available.

## C ABI Hardening

- Document platform ABI assumptions.
- Add more ABI-focused golden tests.
- Add C harnesses for more examples.
- Validate struct layout expectations where practical.
- Improve guidance for host language bindings.

## Python and Node Examples

- Add minimal Python loading example.
- Add minimal Node.js loading example.
- Document 64-bit integer handling, especially JS `BigInt`.
- Keep examples runtime-free on the IntKernel side.

## Benchmarking

- Add repeatable microbenchmarks for generated kernels.
- Compare generated C builds across optimization levels.
- Track benchmark inputs and host compiler versions.

## Phase 10 Checked Arithmetic

Phase 10 checked arithmetic is complete for the current V0 language surface.

- `--overflow unchecked` remains the default.
- `--overflow checked` emits checked C/header output with `IK_Status`.
- Checked mode reports add, subtract, multiply, divide, modulo, and unary minus
  arithmetic failures.
- Checked mode propagates errors across IntKernel function calls.
- Checked mode preserves `&&` and `||` short-circuit behavior.
- Checked mode supports V0 control flow, pointer indexing, and struct field
  access.
- Checked mode does not add bounds checks or user pointer validation.
- Python, Node.js, and benchmark examples include checked-mode entry points.

Future checked-arithmetic work:

- Add a portable overflow fallback for compilers without Clang/GCC
  `__builtin_*_overflow` support.
- Add native MSVC-specific checked arithmetic lowering if the project supports
  MSVC without clang-compatible builtins.
- Keep unchecked overflow as the default unless a future major version changes
  that contract explicitly.

## Phase 11 Typed IR / MIR

Phase 11 Typed IR / MIR is complete for the current V0 language surface. MIR v1
is typed, three-address, and basic-block based, but not SSA.

- `docs/MIR.md` documents MIR v1.
- MIR types, printer, and validator are implemented.
- Typed AST lowers to MIR without changing language semantics.
- MIR-to-C unchecked code generation is implemented.
- MIR-to-C checked code generation is implemented.
- `ikc emit-mir` exposes stable MIR text for compiler debugging.
- The default `emit-c` and `build` pipeline now uses MIR.
- The old AST C backend remains as a legacy/internal fallback during migration.

At Phase 11, MIR v1 did not include an optimizer, register allocation, bounds
checks, runtime support, or new language features. Phase 14 later adds a
conservative MIR optimizer while keeping those safety boundaries.

## Phase 12 WASM Backend

Phase 12 WASM backend is complete for the current V0 language surface covered
by MIR.

- `docs/WASM_ABI.md` documents the WASM ABI.
- The target is `wasm32`.
- `ptr<T>` maps to `i32` linear-memory offsets.
- Module memory is exported as `(memory (export "memory") 1)`.
- Struct layout is deterministic and independent of host C compilers.
- MIR-to-WAT code generation has stable snapshots.
- `ikc emit-wat` emits stable WAT text.
- `ikc emit-wasm` assembles WAT through the bundled `wabt` npm package.
- Node.js and browser WebAssembly examples use `DataView` and `BigInt`.
- `pricing.ik` has WASM e2e coverage.
- The benchmark harness includes an unchecked WASM benchmark.

Phase 12 v1 remains unchecked-only. `--overflow checked` for WASM must report a
clear unsupported-mode error until checked WASM lowering is designed.

Phase 12 does not add WASI, imports, an allocator, runtime support, strings,
bounds checks, `slice<T>`, SIMD, threads, GC, or exceptions.

Future WASM work:

- checked WASM arithmetic
- a simple optional WASM allocator
- richer host-language examples
- WASI integration if a future use case needs imports or host services
- `slice<T>` / bounds-check support if the language gains length-carrying
  pointer types

## Phase 13 LLVM Backend

Phase 13 LLVM backend is complete for the current MIR-supported unchecked V0
language surface.

- `docs/LLVM_BACKEND.md` documents the LLVM backend contract.
- MIR-to-LLVM IR text generation is implemented.
- `ikc emit-llvm` emits stable `.ll` output.
- `ikc build-llvm` can build dynamic libraries through clang.
- `ikc build-llvm --kind object` can emit object files through clang.
- LLVM IR snapshots cover scalar, control flow, function calls,
  ptr/index/field/store, short-circuiting, and `pricing`.
- LLVM clang e2e tests cover scalar, bool ABI, control flow, function calls,
  short-circuiting, memory access, and `pricing`.
- C/WASM/LLVM backend regression comparison tests cover scalar, control flow,
  function call, short-circuit, memory, and pricing fixtures.
- LLVM v1 remains unchecked-only.
- `--overflow checked` is rejected for LLVM until checked LLVM lowering is
  designed.

Phase 13 v1 does not add the LLVM C++ API, bitcode writing, JIT, LLVM-specific
optimizer pipeline, debug info, runtime support, allocator, bounds checks,
`slice<T>`, strings, IO, or modules.

Future LLVM work:

- checked LLVM arithmetic
- broader direct SSA LLVM lowering
- target data layout hardening
- object/static library improvements
- debug info
- JIT maybe, if a future product use case justifies it
- `slice<T>` / bounds check after the language has length-carrying pointer
  types

## Phase 14 Optimization and Performance

Phase 14 optimization and performance work is complete for v0.4.0.

- `ikc` supports `--opt-level 0`, `--opt-level 1`, `--opt-level 2`, and
  `--opt-level 3`, plus `-O0` through `-O3` aliases.
- `-O0` remains the conservative default and keeps output closest to lowered
  MIR.
- `-O1`, `-O2`, and `-O3` enable the documented conservative MIR pass layers.
- Checked C keeps business overflow and division checks; only proven-safe loop
  induction increments can use the checked C hot-path optimization.
- WASM and LLVM remain unchecked-only and reject `--overflow checked`.
- The performance suite supports quick/full runs, private baselines, compare
  mode, and explicit regression guards.
- Optimizations must preserve checked/unchecked semantics and generated ABI, and
  must not specialize for `examples/pricing.ik`.

Future optimization work:

- broader WASM structured control-flow lowering
- broader direct SSA LLVM lowering for scalar control flow
- target data layout hardening
- optional CPU-native/LTO experiments outside default builds
- broader f64 optimization only after a future phase explicitly designs
  strict-safe floating point optimization rules

Numeric roadmap lock:

- IK / IntKernel remains f64-only for floating point.
- `f32` is not planned.
- Phase 20 starts explicit numeric casts with exact `i32_to_f64` and
  `u32_to_f64` builtins only.
- `i64_to_f64`, `u64_to_f64`, f64-to-int casts, overloaded casts, and cast
  expression syntax remain future design work.
- Fast-math and SIMD are not part of the current numeric roadmap.

## Future `slice<T>` / Bounds Checks

- Raw `ptr<T>` remains unchecked.
- Bounds checks should wait for a length-carrying type such as future
  `slice<T>` or explicit pointer-plus-length metadata.
- Document ownership, nullability, and aliasing rules before introducing
  bounds-safe lowering.
