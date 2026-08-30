# CalcKernel 0.11 入门

[English](../../guides/getting-started.md)

End user 解压对应 host 的 release archive 后即可运行 self-contained compiler：

```sh
ckc --version --verbose
ckc check examples/core/scalar.ck
ckc run examples/native/hello.ck
ckc emit-kir examples/core/scalar.ck --print-facts
```

CK 文件包含 struct 与 typed function；exported function 成为 host entry：

```ck
export fn add(a: i32, b: i32) -> i32 {
  return a + b;
}
```

先使用 `check`，再参考 [backend 选择](backend-selection.md)。Release `ckc` 的 `run`/`build`
不需要外部 compiler。Source diagnostic 包含稳定 `CKxxxx` ID、file、line、column、excerpt
与 caret。从源码构建 Native feature 需要固定 LLVM prefix；精确 bootstrap 命令见 README。

MSVC host 的 `.cargo/config.toml` 为两个受支持 Windows target 默认设置
`-C target-feature=+crt-static`，包括 tests 和 debug builds。若覆盖 Cargo Rust flags，
必须保留该 feature；Native build 在编译 bridge 前拒绝动态 CRT。使用 bootstrap 产出的
prefix 并执行 `scripts/validate-llvm-prefix.ps1` 校验，不能换成 `/MD` 或 debug CRT 的
LLVM archives。Native tests 需要 pinned Clang oracle，包括本机架构的真实 COFF archive
检查。

Optimizer contract 是显式 unsafe boundary。只能在 `unsafe { ... }` 中调用 `unsafe fn`，并在
entry 满足全部 requirement。Native run/executable 调试可使用 `--sanitize-contracts`；普通
release 信任 contract，不插入检查。

仓库治理测试还会执行跨平台 CI prefix verifier，因此开发/测试 host 需要 PowerShell 7
（`pwsh`）。它只属于测试工具依赖，构建出的 compiler 与 CK 程序不依赖 PowerShell。

开发门禁：

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
```
