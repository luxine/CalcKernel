# CalcKernel Compiler Architecture

[简体中文](zh-CN/COMPILER_ARCHITECTURE.md)

CalcKernel is a source-to-C, source-to-WASM, and source-to-LLVM-IR compiler
implemented in TypeScript. All production code generation paths consume
validated MIR.

## Pipeline

Current pipeline:

```text
.ck source
  -> SourceFile
  -> lexer
  -> tokens
  -> parser
  -> AST
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> C backend + header emitter
       -> .c / .h
       -> optional build command
       -> dynamic library
  -> WASM backend
       -> .wat
       -> .wasm
  -> LLVM backend
       -> .ll
       -> optional build-llvm command
       -> object file or dynamic library
```

The default native-library path still emits readable C. Native compilation is
delegated to an external C compiler such as clang. The WASM path emits WAT and
assembles it to `.wasm` through `wabt`. The LLVM path emits textual `.ll` and
can invoke clang through `build-llvm`.

Phase 12 adds an implemented WASM path after MIR:

```text
.ck source
  -> lexer
  -> parser
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> MIR WAT backend
  -> .wat
  -> WAT-to-WASM assembly with wabt
  -> .wasm
```

The C backend remains the default native backend. WASM and LLVM are optional
MIR-based backends for users that need `.wasm`, `.ll`, object output, or
LLVM-driven native builds.

## Layer Responsibilities

### SourceFile

`SourceFile` owns the file name and source text. It is passed through lexer,
parser, type checker, diagnostics formatting, and CLI reporting.

### Lexer

The lexer converts raw `.ck` text into tokens. Each token records:

- kind
- text
- line
- column
- start offset
- end offset

The lexer skips whitespace and `//` line comments. Illegal characters produce
diagnostics and lexing continues so callers can report more than one error when
possible.

### Parser

The parser consumes tokens and builds the AST. It is a recursive-descent parser
with precedence parsing for expressions. AST nodes carry source spans so later
phases can report useful errors.

Parser diagnostics use source spans and are preserved for the type checker and
CLI.

### AST

The AST represents declarations, types, statements, and expressions in the V0
language. It intentionally models only V0 features. Unsupported language
features are not represented.

### Type Checker

The type checker builds symbol tables for structs, functions, parameters, and
locals. It validates names, types, assignments, function calls, control-flow
conditions, return types, pointer indexing, and struct field access.

### CheckedProgram / Typed Program

After a successful type check, the compiler exposes a `CheckedProgram` typed
contract. It keeps:

- the original AST
- struct and function symbol information
- parameter, local, and return types
- expression type information
- struct field information

MIR lowering reads this contract instead of reaching into checker internals. The
AST remains the source-shaped syntax tree; `CheckedProgram` is the typed view of
that tree used by later phases.

### MIR Lowering

Phase 11 introduces a Typed MIR layer after the type checker. MIR lowers the
Typed AST into a typed, three-address, basic-block based representation. It
normalizes control flow and lvalue/rvalue handling for backend consumption while
preserving source semantics.

MIR lowering is responsible for:

- turning `if` / `else` and `while` into labeled basic blocks
- lowering `&&` and `||` into control flow so short-circuit behavior is kept
- lowering function calls into explicit `call` instructions
- lowering pointer indexing and struct field access into typed places
- emitting stable temporary and block names for snapshots

### MIR Validator

The MIR validator checks the lowered module before C emission. It validates
function names, block labels, terminators, branch targets, return types, operand
types, function call signatures, and load/store places.

If the default pipeline produces invalid MIR, that is treated as an internal
compiler error. User source errors should already have been reported by the
lexer, parser, or type checker.

### MIR Optimization and C Backend

The default code generation pipeline now lowers the checked program to MIR,
runs the selected conservative MIR optimization pipeline, validates the MIR, and
then emits the target backend. The legacy AST-to-C emitter remains in the
codebase for comparison and fallback while the MIR backend is hardened.

MIR v1 is not SSA. Phase 14 adds conservative MIR optimization levels, but those
passes must preserve checked/unchecked semantics, ABI, and observable language
behavior. The optimizer does not add bounds checks, a runtime, or new language
features. See [MIR](MIR.md) and [Optimization](OPTIMIZATION.md) for the MIR v1
design and pass pipeline.

The MIR C backend emits the `.c` implementation file. It supports both overflow
modes:

- unchecked mode emits ordinary C expressions and original return types
- checked mode emits `CK_Status`, checked arithmetic guards, checked function
  call propagation, and `ik_return` handling

Exported functions are declared in the header. Non-exported functions are
emitted as `static` in the C source.

### Header Emitter

The header emitter is shared by the default MIR pipeline. It emits `.h` files
with:

- `#pragma once`
- standard includes
- `CK_API` and `CK_BUILD_DLL` handling
- C++ `extern "C"` guards
- struct typedefs
- exported function declarations

Unchecked headers keep original return types. Checked headers include
`CK_Status` and add the final `ik_return` pointer to exported function
signatures.

### Build Command

The CLI `build` command emits C/header files and invokes clang with strict flags:

```text
-std=c11 -O3 -Wall -Wextra -Werror
```

V0 does not bundle a runtime or compiler toolchain.

### WASM Backend

The Phase 12 WASM backend consumes validated MIR and emits stable WAT. The
`emit-wasm` command assembles that WAT to a `.wasm` binary through the bundled
`wabt` npm package. The target ABI is `wasm32`: `ptr<T>` becomes an `i32`
linear-memory offset, `bool` uses `i32`, and `i64` / `u64` are exposed to
JavaScript as `BigInt`.

Phase 12 v1 is intentionally narrow:

- unchecked arithmetic only
- exported linear memory
- deterministic CalcKernel struct layout
- scalar expressions, control flow, short-circuiting, function calls, and
  ptr/index/field load/store patterns
- no WASI imports
- no allocator
- no runtime
- no bounds checks

See [WASM ABI](WASM_ABI.md) for the Phase 12 ABI and usage model.

### LLVM Backend

The Phase 13 LLVM backend consumes validated MIR and emits stable textual LLVM
IR (`.ll`). It intentionally does not embed LLVM libraries or use the LLVM C++
API in v1.

The LLVM path is:

```text
.ck / .ck source
  -> AST
  -> CheckedProgram / Typed Program
  -> MIR
  -> MIR validator
  -> LLVM IR text backend
  -> .ll
  -> optional clang / llc build
```

Phase 13 v1 is unchecked-only, alloca/load/store based, and focused on scalar
operations, control flow, function calls, and ptr/index/field load/store. It
does not add an LLVM-specific optimizer pipeline, checked LLVM lowering, JIT,
debug info, runtime, allocator, bounds checks, or `slice<T>`.

`emit-llvm` does not require clang. `build-llvm` invokes external clang to build
a dynamic library or object file. See [LLVM Backend](LLVM_BACKEND.md) for the
backend contract and limitations.

## Diagnostics Flow

Diagnostics are collected as data and flow through the pipeline:

```text
lexer diagnostics
  -> parser diagnostics
  -> type checker diagnostics
  -> CLI formatter
```

Each diagnostic contains:

- error code
- severity
- message
- file name
- line
- column
- source span

The CLI formats diagnostics with file location, code, message, source line, and
a caret range.

MIR validator failures are reported as internal compiler errors. They indicate a
compiler bug after type checking rather than a user source-language diagnostic.

## Why C First

The original V0 compiler emitted C before adding MIR and WASM for pragmatic
reasons:

- C is easy to inspect and review.
- C ABI integration is widely supported by Node.js, Python, Java, Rust, Go, C#,
  and other host languages.
- Existing platform C compilers already handle native optimization and dynamic
  library generation.
- It keeps the compiler small while the language and ABI stabilize.

WASM becomes an optional MIR backend in Phase 12. LLVM becomes an optional MIR
backend in Phase 13. C remains the default native backend, while regression
tests compare C, WASM, and LLVM behavior across scalar, control-flow,
function-call, short-circuit, memory, and pricing fixtures.

## Future IR Direction

Before Phase 11, the backend emitted C directly from the checked AST. Phase 11
adds a Typed MIR layer with this long-term direction:

- MIR for a simpler, normalized typed program representation
- MIR for control-flow lowering and backend-independent code generation
- backend-specific lowering for C, WASM, or LLVM

MIR v1 is intentionally conservative: no SSA, no register allocation, no bounds
checks, no runtime, and no new language features. Optimization is limited to the
documented MIR pass manager and must prioritize correctness over performance.
