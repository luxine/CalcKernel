# Getting Started with CalcKernel 0.10

[简体中文](../zh-CN/guides/getting-started.md)

For end users, unpack the release archive for the host and run the self-contained
compiler:

```sh
ckc --version --verbose
ckc check examples/core/scalar.ck
ckc run examples/native/hello.ck
```

A CK file contains structs and typed functions. Exported functions become host
entry points:

```ck
export fn add(a: i32, b: i32) -> i32 {
  return a + b;
}
```

Use `check` first, then select an output with the [backend guide](backend-selection.md).
Release `ckc` needs no external compiler for `run` or `build`. Source diagnostics
include a stable `CKxxxx` identifier, file, line, column, excerpt, and caret.

Building the Native feature from source requires the pinned LLVM prefix; follow
the exact bootstrap command in the repository README. Default features remain
available for frontend/C/WASM development without that prefix.

For development, run the strict gate:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
```
