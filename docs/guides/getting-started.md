# Getting Started with CalcKernel 0.9

[简体中文](../zh-CN/guides/getting-started.md)

Install stable Rust, clone the repository, and build the native compiler:

```sh
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/core/scalar.ck
./target/release/ckc emit-mir examples/core/scalar.ck -O3
```

A CK file contains structs and typed functions. Exported functions become host
entry points:

```ck
export fn add(a: i32, b: i32) -> i32 {
  return a + b;
}
```

Use `check` first, then select an output with the
[backend guide](backend-selection.md). `emit-*` needs only `ckc`; `build` and
`build-llvm` also need `clang`. Source diagnostics include a stable `CKxxxx`
identifier, file, line, column, source excerpt, and caret.

For development, run the strict gate:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```
