# ckc 产物与依赖

[English](../ckc-outputs.md)

本文说明 `native ckc` 可以生成哪些 artifact，以及使用这些 artifact 需要哪些工具。

## 命令

| 命令 | 输出 | 生成依赖 | 后续使用依赖 |
| --- | --- | --- | --- |
| `ckc check input.ck` | 无 | `ckc` | 无 |
| `ckc emit-mir input.ck` | MIR text | `ckc` | 无 |
| `ckc emit-c input.ck -o input.c` | C source 和 header | `ckc` | C compiler |
| `ckc emit-wat input.ck -o input.wat` | WAT text | `ckc` | 无 |
| `ckc emit-wasm input.ck -o input.wasm` | WASM binary | `ckc` | WebAssembly runtime |
| `ckc emit-llvm input.ck -o input.ll` | LLVM IR text | `ckc` | LLVM tools |
| `ckc build input.ck -o output` | dynamic library | `ckc` 和 clang | host loader / FFI |
| `ckc build-llvm input.ck --kind object -o input.o` | object file | `ckc` 和 clang | linker |

生成依赖和使用依赖是两回事。例如 `emit-c` 只需要 `ckc` 写出 C 和 header，但继续
编译这些 C 文件需要 C compiler。

## 原生 CLI

```sh
cargo build --release --locked
./target/release/ckc --help
./target/release/ckc check examples/scalar.ck
```

已安装的 release artifact 可以直接用 `ckc` 调用。

## MIR

```sh
ckc emit-mir examples/scalar.ck -O3
ckc emit-mir examples/scalar.ck -O3 -o build/scalar.mir
```

MIR 是 compiler debugging artifact，适合 lowering、optimizer 和 snapshot 工作。
它不能直接执行。

## C

```sh
ckc emit-c examples/pricing.ck --out build/pricing.c --header build/pricing.h
clang -O3 -c build/pricing.c -o build/pricing.o
```

C 路线适合 native integration、C ABI、FFI，以及把 CK kernel 嵌入服务。C backend
支持 checked overflow mode。

## WASM

```sh
ckc emit-wat examples/wasm_scalar.ck --out build/scalar.wat
ckc emit-wasm examples/wasm_scalar.ck --out build/scalar.wasm
```

`emit-wasm` 写出 WebAssembly binary。该 binary 需要 WebAssembly runtime 实例化。
生成的 module 遵循 caller-owned memory 规则：host 选择 offset，把 input 写入
linear memory，调用 exported function，再读回 output。

推荐 host pattern：

- homogeneous buffer 使用 typed-array view
- mixed-width struct 和 byte-exact ABI 检查使用 `DataView`
- `memory.grow` 后重新创建 view
- 除非 kernel 明确设计为 in-place update，否则保持 input/output region 不重叠

## LLVM

```sh
ckc emit-llvm examples/llvm_scalar.ck --out build/scalar.ll
ckc build-llvm examples/llvm_scalar.ck --kind object --out build/scalar.o
```

`emit-llvm` 写出 textual LLVM IR，不需要 clang。`build-llvm` 会调用 clang，并根据
`--kind` 生成 object file 或 dynamic library。

## 当前不支持的直接输出

当前 native CLI 不直接提供：

- 从 `ckc emit-*` 直接生成 standalone executable
- 无工具链依赖的 `.so` / `.dylib` / `.dll` generator
- Python wheel
- Java jar
- GPU、CUDA 或 WebGPU shader output
- SIMD WASM output
- f32 kernel
- `ckc run input.ck`

部分 native library flow 已通过 `ckc build` 和 `ckc build-llvm` 存在，但这些命令
仍依赖平台工具链。

## Slice 与 bounds output matrix

所有 emitter 都能 lower 合法 `slice<T>` program。`ckc emit-c` 与 `ckc build`
接受 `--overflow unchecked|checked` 和 `--bounds unchecked|checked` 的全部组合；
任一 checked mode 都选择完整五状态 ABI。`ckc emit-wat`、`emit-wasm`、
`emit-llvm` 与 `build-llvm` 接受 `--bounds unchecked`，并以稳定的 backend-specific
help 拒绝 checked bounds。`check` 与 `emit-mir` 属于 source-semantic command，
不消费 backend bounds mode。

C header 为 stored field 暴露 typed descriptor，但展平 slice parameter。WASM
artifact 暴露 address/length pair。LLVM IR 暴露展平的 `ptr, i32` parameter 与
`{ ptr, i32 }` stored value。
