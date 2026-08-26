# CalcKernel 0.9 入门

[English](../../guides/getting-started.md)

安装 stable Rust，clone 仓库并构建原生 compiler：

```sh
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/scalar.ck
./target/release/ckc emit-mir examples/scalar.ck -O3
```

CK 文件包含 struct 与 typed function；exported function 成为 host entry：

```ck
export fn add(a: i32, b: i32) -> i32 {
  return a + b;
}
```

先使用 `check`，再参考 [backend 选择](backend-selection.md)。`emit-*` 只依赖
`ckc`；`build` / `build-llvm` 还依赖 `clang`。Source diagnostic 包含稳定
`CKxxxx` ID、file、line、column、source excerpt 与 caret。

开发门禁：

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```
