# WASM Interop

[English](../wasm-interop.md)

本文说明如何使用 `native ckc` 生成的 WASM artifact。

## 生成 Module

```sh
cargo build --release --locked
./target/release/ckc emit-wasm examples/wasm_scalar.ck --out build/scalar.wasm
./target/release/ckc emit-wat examples/wasm_scalar.ck --out build/scalar.wat
```

`emit-wasm` 生成 WebAssembly binary。运行该 binary 需要 host environment 提供
WebAssembly runtime。

## Memory Model

生成的 module 使用 caller-owned memory。host 在 exported linear memory 中选择
offset，写入 input buffer，调用 CK function，再读取 output buffer。CK 不提供
allocator 或 runtime。

host 侧规则：

- `ptr<T>` value 是 byte offset
- 遵守 `docs/WASM_ABI.md` 中的 layout
- 使用 little-endian read/write
- homogeneous array 优先使用 typed-array view
- mixed-width struct 和 byte-level test 使用 `DataView`
- `memory.grow` 后重新创建 view

## 验证

```sh
cargo test --test wasm_backend_test --locked
```

测试套件检查 WAT text、WASM bytes、ABI layout、f64 behavior、fixture coverage，
以及生成 artifact 的 runtime behavior。

## Slice interop

Exported `slice<T>` parameter 对 host 表现为两个 argument：linear-memory address
与 `u32` element count。Descriptor 非 owning、可以 alias，也不会保持 memory
alive；host 必须保证整个 call 期间 allocation 有效。Stored descriptor 占 8 bytes，
字段 offset 为 0/4。Internal multi-value return 属于 compiler detail，exported
slice return 会被拒绝。

WASM 当前只支持 `--bounds unchecked`，并明确拒绝 `--bounds checked`。需要
recoverable checked slice error 的 host 应使用 generated C；host 也可以在调用 WASM
前自行 validation。通过 `.data` index 在所有 backend 都是 raw escape。
