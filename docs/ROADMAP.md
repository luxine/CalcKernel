# IntKernel Roadmap

This roadmap tracks likely work after V0. It is not a promise that every item
will ship in this order.

## V0 Stable

- Keep the language intentionally small.
- Stabilize lexer, parser, type checker, diagnostics, C backend, CLI, and tests.
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

MIR v1 explicitly does not include an optimizer, constant folding, dead code
elimination, register allocation, bounds checks, runtime support, or new
language features.

## Phase 12 WASM Backend

- Explore a WASM backend after the C ABI and V0 semantics are stable.
- Define pointer, memory, and host integration rules explicitly.
- Decide whether WASM should consume MIR directly or use a WASM-specific lower
  layer.
- Keep V0's no-runtime and caller-owned-memory constraints unless a separate
  design explicitly changes them.

## Phase 13 LLVM Backend

- Explore LLVM only after the language and IR are stable enough to justify the
  added complexity.
- Keep C backend as the reference backend until another backend is proven.
- Define how checked arithmetic maps to LLVM intrinsics or explicit checks.

## Future Optimizer

- Consider a separate optimization phase only after MIR remains stable across at
  least one release.
- Candidate passes include constant folding, dead code elimination, common
  subexpression elimination, and range analysis.
- Any optimizer must preserve checked/unchecked semantics and generated ABI.

## Future `slice<T>` / Bounds Checks

- Raw `ptr<T>` remains unchecked.
- Bounds checks should wait for a length-carrying type such as future
  `slice<T>` or explicit pointer-plus-length metadata.
- Document ownership, nullability, and aliasing rules before introducing
  bounds-safe lowering.
