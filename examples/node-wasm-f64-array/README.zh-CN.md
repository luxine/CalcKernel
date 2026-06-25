# Node.js WASM f64 Float64Array 示例

[English](README.md)

这个示例展示如何在 Node.js 中调用 IK / IntKernel 生成的 WASM `f64`
kernel，并用指向 exported WASM memory 的 `Float64Array` view 读写
`ptr<f64>` buffer。对于已经是 numeric 且 8-byte aligned 的批量 `f64`
数据，这是推荐的 host 侧路径。

这个示例不会引入 IK runtime 或 allocator。byte offset、memory grow 和
typed-array view 重建都由 host 负责。

## 生成 WASM

从仓库根目录先构建本地 CLI：

```sh
pnpm build
```

生成 `build/f64_array.wasm`：

```sh
ikc emit-wasm examples/node-wasm-f64-array/f64_array.ik --out build/f64_array.wasm -O3
```

源码 checkout 中也可以通过 pnpm 运行：

```sh
pnpm ikc emit-wasm examples/node-wasm-f64-array/f64_array.ik --out build/f64_array.wasm -O3
```

## 运行

从仓库根目录运行：

```sh
node examples/node-wasm-f64-array/index.mjs
```

也可以指定自定义 WASM 路径：

```sh
node examples/node-wasm-f64-array/index.mjs --wasm build/f64_array.wasm
```

当 `axpy_f64` 的 output buffer 在 tolerance 范围内匹配预期值时，脚本会输出
`OK`。

## ptr<f64> 规则

WASM `ptr<f64>` 是 `i32` byte offset：

- `f64` size 是 8 bytes。
- `ptr<f64>[i]` 的 byte offset 是 `base + i * 8`。
- `Float64Array` index 是 `byteOffset / 8`。
- `byteOffset` 必须 8-byte aligned。

hot path 使用 `Float64Array#set` 和 `Float64Array#subarray`，不在循环中逐元素
调用 `DataView.setFloat64` / `DataView.getFloat64`。

`DataView` 仍适合 byte-level ABI 测试和 mixed-width struct packing。
`Float64Array` 是 homogeneous `f64` buffer 的批量推荐路径。

## Memory Ownership

IK / IntKernel 当前没有 WASM allocator、runtime 或 bounds checks。host 必须负责
memory placement，并确保 input/output 区域有效。

如果调用了 `memory.grow`，旧的 `Float64Array` view 可能失效。任何 grow 之后都要
重新创建 typed-array view：

```js
const values = new Float64Array(memory.buffer);
```

不要承诺这个路径总是比 `DataView` 更快；实际取决于数据来源和 host 是否本来就需要
copy。对于大块 homogeneous `f64` 数组，它是推荐 baseline。
