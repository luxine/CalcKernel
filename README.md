# Rust CalcKernel

[Simplified Chinese](README.zh-CN.md)

Rust CalcKernel ships `native ckc`: a Rust-built command-line compiler for the
CK / CalcKernel language. This repository no longer publishes a wrapper layer
or a scripting-language package surface. The product boundary is the native
`ckc` executable plus the Rust compiler implementation behind it.

## Project Shape

CK is a small DSL for pure computation kernels. It is intended for deterministic
logic such as pricing, array processing, graph algorithms, and numerical
kernels that need embeddable C, WASM, or LLVM outputs.

This repository provides:

- Rust lexer, parser, type checker, MIR lowering, and MIR optimization passes.
- A native `ckc` CLI with `check`, `emit-mir`, `emit-c`, `emit-wat`,
  `emit-wasm`, `emit-llvm`, `build`, and `build-llvm`.
- C backend output with C/H generation, unchecked and checked overflow ABI
  modes, and optional `clang` shared-library builds.
- WASM backend output as WAT or WASM bytes.
- LLVM backend output as LLVM IR, dynamic libraries, or object files through
  `clang`.
- Native release artifacts documented in `docs/native-release.md`.

## Architecture

```text
.ck source
  |
  v
lexer -> parser -> type checker
  |
  v
MIR lowering -> MIR optimizer
  |
  +--> C emitter -> clang -> shared library
  +--> WAT emitter -> wasm bytes
  +--> LLVM IR emitter -> clang -> shared library / object
```

Primary source entry points:

- `src/lexer/mod.rs`: tokenization, source positions, and lexer diagnostics.
- `src/parser.rs`: AST, statements, expressions, and parser diagnostics.
- `src/typeck.rs`: symbol tables, scopes, type checking, and metadata lookup
  helpers.
- `src/mir/mod.rs`: MIR data structures, lowering, validation, and printing.
- `src/opt/mod.rs`: O0-O3 pass pipeline and MIR optimization passes.
- `src/backend/mod.rs`: C, WAT/WASM, and LLVM backends.
- `src/main.rs`: native `ckc` CLI argument parsing, file IO, `clang` calls,
  stdout/stderr, and exit codes.

## Usage

Build and run the native CLI:

```sh
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/scalar.ck
./target/release/ckc check examples/control_flow.ck
./target/release/ckc emit-mir examples/scalar.ck -O3
./target/release/ckc emit-c examples/pricing.ck --out /tmp/pricing.c
./target/release/ckc emit-wasm examples/wasm_scalar.ck --out /tmp/scalar.wasm
./target/release/ckc emit-llvm examples/llvm_scalar.ck --target ck-test-target
```

During development, `cargo run --` can be used in place of the release binary:

```sh
cargo run -- check examples/scalar.ck
```

## Documentation

- `docs/LANGUAGE_SPEC.md`: CK source language.
- `examples/control_flow.ck`: `break` / `continue` control-flow example.
- `docs/COMPILER_ARCHITECTURE.md`: compiler pipeline and module boundaries.
- `docs/MIR.md`: MIR data model and printed format.
- `docs/OPTIMIZATION.md`: MIR optimization levels and pass boundaries.
- `docs/ABI.md`, `docs/WASM_ABI.md`, and `docs/LLVM_BACKEND.md`: backend ABI
  contracts.
- `docs/ckc-outputs.md`: output files and when to use each backend.
- `docs/native-release.md`: native release process and artifact checks.

Formal user-facing docs have matching Simplified Chinese versions under
`docs/zh-CN/`.

## Verification

The strict local gate is:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/scalar.ck
```

The Rust tests preserve compiler behavior against the read-only TypeScript
source checkout where useful. Those oracle tests compare native compiler
behavior: diagnostics, stdout/stderr, exit codes, MIR text, generated C/WASM/LLVM
outputs, and runtime behavior for emitted artifacts. They do not protect a
wrapper API or a registry publication path.

## Release Boundary

Release builds produce native `ckc` binaries for supported macOS, Linux, and
Windows targets. Each archive is signed off with CLI smoke checks and a `SHA256`
checksum. Tagged builds may attach those archives to a `GitHub Release`.

No npm. No JavaScript compatibility layer. No TypeScript declaration parity.
The TypeScript checkout remains read-only source material and behavior oracle;
the shipped product is `native ckc`.
