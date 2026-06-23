# Node.js WASM Pricing 示例

[English](README.md)

这个示例使用 Node.js 内置 WebAssembly API 调用从 `examples/pricing.ik`
生成的 WebAssembly module。它不需要 native `.so`、`.dylib` 或 `.dll`，
也不需要安装示例目录内的额外依赖。

## 生成 WASM

先在仓库根目录构建本地 CLI：

```sh
pnpm build
```

然后从当前示例目录生成 `../../build/pricing.wasm`：

```sh
ikc emit-wasm ../../examples/pricing.ik --out ../../build/pricing.wasm
```

如果是在源码 checkout 中运行，也可以从当前目录通过 pnpm 调用同一个 CLI：

```sh
pnpm --dir ../.. ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
```

## 运行

从仓库根目录运行：

```sh
node examples/node-wasm-call/index.mjs
```

或者从当前目录运行：

```sh
node index.mjs
```

当 `calc_items` 返回 `0` 且 output buffer 与预期值一致时，脚本会打印 `OK`。

## ABI 映射

生成的 WASM 导出：

- `memory`：一个 WebAssembly linear memory。
- `calc_items(items: i32, len: i32, out: i32) -> i32`。

`ptr<T>` 是一个数字 memory offset。宿主侧通过在 `memory.buffer` 中选择 offset，
手动分配输入和输出区域。

`i64` 和 `u64` 使用 JavaScript `BigInt`。不要把较大的 64-bit 值当作
JavaScript `number` 传递。

宿主侧用 `DataView` 写入 memory。WebAssembly memory 是 little-endian，所以
本示例中所有 `DataView` 读写都为 `littleEndian` 参数传入 `true`。

## Item Layout

`pricing.ik` 定义：

```ik
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}
```

WASM layout：

| Field | Offset | Type |
| --- | ---: | --- |
| `price` | 0 | `i64` |
| `qty` | 8 | `i64` |
| `discount` | 16 | `i64` |
| `tax_rate_ppm` | 24 | `i64` |

`sizeof(Item) = 32`。

调用方负责分配输入 `Item` 数组和输出 `i64` buffer。

## 安全说明

WASM v1 是 unchecked：

- 不做 bounds check
- 不做 checked overflow
- 不检查 pointer validity
- 不提供 allocator
- 不引入 runtime

调用方必须保证 `items`、`len` 和 `out` 描述的是有效 memory region。
