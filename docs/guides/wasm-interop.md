# WebAssembly Host Interop

[简体中文](../zh-CN/guides/wasm-interop.md)

Generate a module with:

```sh
ckc emit-wasm examples/wasm/scalar.ck --out build/scalar.wasm
```

The WebAssembly runtime owns instantiation; the caller owns memory contents.
Choose non-overlapping byte regions, write input values in little-endian form,
call the exported function, then read results. Use typed arrays for homogeneous
buffers and `DataView` for mixed-width structs or exact layout checks. Recreate
views after `memory.grow` because the backing buffer may change.

Pointers are `i32` byte offsets. A `ptr<f64>` index advances by 8 bytes. A
`slice<T>` argument is passed as address then `u32` length. Stored slice
descriptors use address at offset 0 and length at offset 4. The declared length
does not validate allocation extent; memory remains caller-owned and descriptors
may alias.

WASM uses `--bounds unchecked` and `--overflow unchecked`. The CLI will reject
checked modes; no implicit trap or guard is inserted. Validate offsets and
lengths in the host when untrusted input reaches an export. See the normative
[WASM ABI](../abi/wasm.md).
