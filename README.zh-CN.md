# Rust CalcKernel

[English](README.md)

Rust CalcKernel 0.11.0 发布 `native ckc`：一个可自包含运行的 CK computation-kernel
语言命令行编译器。Release binary 无需外部 compiler toolchain 即可编译、链接和运行 Native CK；
仓库同时保留可检查的 C 与 WebAssembly emitter。

## 发布能力

- Rust lexer、parser、type checker、deterministic semantic MIR，以及所有 backend 共用的
  单一 verified fact-driven KIR optimizer。
- `unsafe fn` entry contract，覆盖 affine range、alignment、no-alias 与 memory-effect
  ceiling，并提供 opt-in contract sanitizer。
- `break`/`continue`、return-only `void`、caller-owned `slice<T>`、可选 overflow/bounds
  checked mode 与无参数 `main` entry。
- `ckc run` 隔离 child、确定性 numeric/boolean print 与安全 persistent object cache。
- 内嵌 LLVM 22.1.8 codegen 和进程内 LLD 的
  `ckc build --kind executable|dynamic|static|object`。
- Object/static/dynamic library 共用 generated-header Native C ABI。
- Source-only C 与 portable WAT/WASM 输出。
- 面向 macOS、Linux、Windows 的 AArch64/x86-64 六个零工具链 release archive。

Native checked mode 支持 overflow/bounds 四种组合；C emission 使用相同 status semantics；
WebAssembly 仅支持 unchecked。Native runtime print 可用于 `run`/executable，library、C、
WebAssembly root 可达的 print 会被拒绝。

## Pipeline

```text
.ck -> frontend -> semantic MIR -> mode/consumer-specific verified KIR
                                      +-> C source/header
                                      +-> WAT/WASM
                                      +-> structural LLVM -> object
                                                               +-> ORC run
                                                               +-> in-process LLD -> executable/library
```

产品路径不调用 Clang、system linker 或 archiver。固定 Clang 22.1.8 只作为仓库的 differential
与 ABI test oracle。

## 使用 release binary

```sh
ckc --version --verbose
ckc check examples/core/scalar.ck
ckc emit-kir examples/core/scalar.ck --print-facts
ckc run examples/native/hello.ck
ckc build examples/native/hello.ck --kind executable --out /tmp/hello
ckc build examples/core/scalar.ck --kind dynamic --out /tmp/scalar
ckc emit-c examples/applications/pricing.ck --out /tmp/pricing.c
ckc emit-wasm examples/wasm/scalar.ck --out /tmp/scalar.wasm
ckc licenses
```

`run` 与 `build` 默认 O3；Native build 默认 release target 的 portable CPU baseline，
`--cpu native` 为 opt-in。`build-llvm` 仅保留为 dynamic/object Native build 的 deprecated alias。

## 从源码构建

Native feature 需要 `native/llvm/manifest.toml` 定义的精确 LLVM prefix；仓库脚本将其 bootstrap
到 `build/llvm`。该 prefix 是 build input，不是 end-user runtime dependency。

```sh
rustc_host="$(rustc -vV | sed -n 's/^host: //p')"
llvm_archive=/path/to/llvm-project-22.1.8.src.tar.xz
./scripts/bootstrap-llvm.sh --archive "$llvm_archive" \
  --prefix "$PWD/build/llvm/prefix-$rustc_host-release" \
  --target "$rustc_host" --profile release
export CKC_LLVM_PREFIX="$PWD/build/llvm/prefix-$rustc_host-release"
cargo build --release --features native-toolchain --locked
cargo test --all-features --locked
```

Default feature 可构建 frontend/C/WASM-only developer 版本：

```sh
cargo test --locked
cargo build --release --locked
```

## 文档与验证

入口见 [文档索引](docs/zh-CN/index.md)、[语言参考](docs/zh-CN/reference/language.md)、
[CLI 参考](docs/zh-CN/reference/cli.md) 与 [Native ABI](docs/zh-CN/abi/llvm.md)。
语言示例包含 [control flow](examples/core/control_flow.ck)、[void procedure](examples/core/void.ck)
与 [slice](examples/core/slices.ck)。英文 [release policy](docs/project/release.md) 与中文版本保持镜像。

严格 Native local gate：

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
./target/release/ckc --version --verbose
./target/release/ckc licenses
```

Release policy、platform audit、performance gate、archive name 与 immutable GitHub Release
发布见 [release policy](docs/zh-CN/project/release.md)。

CalcKernel 0.11.0 保持 public Native C ABI version 1；private LLVM bridge 与
contract-aware runtime ABI 更新为 version 2。Native cache 使用 KIR v1 code-generation
identity，因此不会与 0.10 cache entry 混用。
已接受的 0.10.0 source boundary 与 migration 保留在
[兼容性策略](docs/zh-CN/project/compatibility.md)中。

## 内存边界

`slice(data, len)` 与 `items[start..end]` 创建 non-owning `slice<T>` descriptor。Raw pointer validity、
allocation extent、alignment、lifetime 与声明 length 仍由 caller 负责。`--bounds checked` 只验证
slice index/range relation，不会让任意 pointer use 变为 memory-safe。
