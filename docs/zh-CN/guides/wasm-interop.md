# WebAssembly Host Interop

[English](../../guides/wasm-interop.md)

```sh
ckc emit-wasm examples/wasm_scalar.ck --out build/scalar.wasm
```

WebAssembly runtime 负责 instantiate，caller 负责 memory 内容。选择不重叠 byte
region，以 little-endian 写 input，调用 export 后读取 output。Homogeneous buffer
用 typed array；mixed-width struct 或精确 layout check 用 `DataView`；
`memory.grow` 后重建 view。

Pointer 是 `i32` byte offset；`ptr<f64>` index 每次前进 8 byte。`slice<T>` argument
按 address、`u32` length 传递；stored descriptor 的 offset 0 为 address、offset 4
为 length。声明 length 不验证 allocation extent；memory 仍 caller-owned，
descriptor 可 alias。

WASM 使用 `--bounds unchecked` 与 `--overflow unchecked`。CLI 拒绝 checked
mode，不插入 implicit trap/guard；untrusted input 必须由 host 验证 offset/length。
规范 contract 见 [WASM ABI](../abi/wasm.md)。
