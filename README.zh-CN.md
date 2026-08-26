# Rust CalcKernel

[English](README.md)

Rust CalcKernel 发布 `native ckc`：一个由 Rust 构建的 CK / CalcKernel
命令行编译器。本仓库不再维护脚本语言包装层或包发布表面；产品边界是原生 `ckc`
可执行文件，以及它背后的 Rust 编译器实现。

## 项目形态

CK 是一个面向纯计算 kernel 的小型 DSL，适合定价、数组处理、图算法和数值计算等
需要生成 C、WASM 或 LLVM 输出的确定性逻辑。

本仓库提供：

- Rust lexer、parser、type checker、MIR lowering 和 MIR optimizer。
- 原生 `ckc` CLI：`check`、`emit-mir`、`emit-c`、`emit-wat`、`emit-wasm`、
  `emit-llvm`、`build`、`build-llvm`。
- C backend：生成 C/H，支持 unchecked 和 checked overflow ABI，并可通过
  `clang` 构建 shared library。
- WASM backend：生成 WAT 或 WASM bytes。
- LLVM backend：生成 LLVM IR，并可通过 `clang` 构建 dynamic library 或 object。
- 原生发布流程，见 `docs/native-release.md`。

## 架构

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

主要入口：

- `src/lexer/mod.rs`：token、source position 和 lexer diagnostics。
- `src/parser.rs`：AST、statement、expression 和 parser diagnostics。
- `src/typeck.rs`：symbol table、scope、type checking 和 metadata lookup helper。
- `src/mir/mod.rs`：MIR 数据结构、lowering、validation 和打印。
- `src/opt/mod.rs`：O0-O3 pass pipeline 和 MIR optimization passes。
- `src/backend/mod.rs`：C、WAT/WASM 和 LLVM backend。
- `src/main.rs`：原生 `ckc` CLI 参数解析、文件 IO、`clang` 调用、stdout/stderr
  和 exit code。

## 使用

构建并运行原生 CLI：

```sh
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/scalar.ck
./target/release/ckc check examples/control_flow.ck
./target/release/ckc check examples/void.ck
./target/release/ckc emit-mir examples/scalar.ck -O3
./target/release/ckc emit-c examples/pricing.ck --out /tmp/pricing.c
./target/release/ckc emit-wasm examples/wasm_scalar.ck --out /tmp/scalar.wasm
./target/release/ckc emit-llvm examples/llvm_scalar.ck --target ck-test-target
```

开发时也可以使用 `cargo run --`：

```sh
cargo run -- check examples/scalar.ck
```

## 文档

- `docs/LANGUAGE_SPEC.md`：CK 源语言。
- `examples/control_flow.ck`：`break` / `continue` 控制流示例。
- `examples/void.ck`：显式 `void`、提前返回和 procedure call 示例。
- `docs/COMPILER_ARCHITECTURE.md`：compiler pipeline 和模块边界。
- `docs/MIR.md`：MIR 数据模型和打印格式。
- `docs/OPTIMIZATION.md`：MIR 优化等级和 pass 边界。
- `docs/ABI.md`、`docs/WASM_ABI.md`、`docs/LLVM_BACKEND.md`：backend ABI contract。
- `docs/ckc-outputs.md`：输出文件以及各 backend 的使用场景。
- `docs/native-release.md`：原生发布流程和 artifact 检查。

正式用户文档都在 `docs/zh-CN/` 下维护简体中文版本。

## 验证

严格本地门禁：

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/scalar.ck
```

Rust 测试会在有价值的地方继续使用只读 TypeScript source checkout 作为行为
oracle。这些测试比较的是原生编译器行为：diagnostics、stdout/stderr、exit code、
MIR 文本、生成的 C/WASM/LLVM 输出，以及生成 artifact 的运行时行为。它们不保护
包装 API，也不保护 registry 发布路径。

## 发布边界

发布构建产出 macOS、Linux 和 Windows 的原生 `ckc` 二进制。每个归档都通过 CLI
冒烟检查和 `SHA256` checksum 签核。带 tag 的构建可以把这些归档附加到
`GitHub Release`。

No npm。没有 JavaScript compatibility layer。没有 TypeScript declaration parity。
TypeScript checkout 只作为只读 source material 和行为 oracle；实际发布的产品是
`native ckc`。

## 非 owning slice

CK 现已支持非 owning 的 `slice<T>` descriptor。可用 `slice(data, len)` 从 raw
pointer 构造，读取 `.data` / `.len`，执行 index，或用 `items[start..end]` 构造
半开 sub-slice。内存仍由 caller 拥有；raw pointer 与声明长度的有效性也由 caller
负责。Descriptor 会 alias 同一内存，不会延长其 lifetime。示例见
[examples/slices.ck](examples/slices.ck)。

所有 backend 默认 `--bounds unchecked`。C emission 与 `ckc build` 还接受
`--bounds checked`，越界返回 `CK_ERR_OUT_OF_BOUNDS`（status 4）。WASM 与 LLVM
只接受 unchecked bounds，并会明确拒绝 checked bounds。
