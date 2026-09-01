# Getting Started with CalcKernel 0.12

[简体中文](../zh-CN/guides/getting-started.md)

For end users, unpack the release archive for the host and run the self-contained
compiler:

```sh
ckc --version --verbose
ckc check examples/core/scalar.ck
ckc run examples/native/hello.ck
ckc emit-kir examples/core/scalar.ck --print-facts
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

Optimization contracts are an explicit unsafe boundary. Call an `unsafe fn`
only inside `unsafe { ... }` and satisfy every entry requirement. During Native
run/executable debugging, add `--sanitize-contracts`; ordinary releases trust
the contract and do not insert checks.

Building the Native feature from source requires the pinned LLVM prefix; follow
the exact bootstrap command in the repository README. Default features remain
available for frontend/C/WASM development without that prefix.

For MSVC hosts, `.cargo/config.toml` enables `-C target-feature=+crt-static`
for both supported Windows targets, including tests and debug builds. If you
override Cargo Rust flags, preserve that feature: Native builds reject a
dynamic CRT setting before compiling the bridge. Use the bootstrap-produced
prefix and validate it with `scripts/validate-llvm-prefix.ps1`; do not substitute
LLVM archives built with `/MD` or a debug CRT. Native tests require the pinned
Clang oracle, including actual COFF archive checks for the host architecture.

Repository governance tests also execute the cross-platform CI prefix verifier;
they require PowerShell 7 (`pwsh`) on the development/test host. This is a test
tool dependency only, not a dependency of the built compiler or CK programs.

For development, run the strict gate:

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
```
