# WebAssembly Host Interop

[English](../../guides/wasm-interop.md)

```sh
ckc emit-wasm examples/wasm/scalar.ck --out build/scalar.wasm
```

WebAssembly runtime 负责 instantiate，caller 负责 memory 内容。选择不重叠 byte
region，以 little-endian 写 input，调用 export 后读取 output。Homogeneous buffer
用 typed array；mixed-width struct 或精确 layout check 用 `DataView`；
`memory.grow` 后重建 view。

Pointer 是 `i32` byte offset；`ptr<f64>` index 每次前进 8 byte。`slice<T>` argument
按 address、`u32` length 传递；stored descriptor 的 offset 0 为 address、offset 4
为 length。声明 length 不验证 allocation extent；memory 仍 caller-owned，
descriptor 可 alias。

## 持久化 host memory

`examples/wasm/host/ck-wasm-arena.mjs` 为导出 `memory` 与 `__ck_heap_base` 的
模块提供单调 allocator。它会在多次调用间保留分配结果、对齐 typed region、检查
长度与 Wasm32 地址运算，并在空间不足时增长 memory。增长失败时分配游标保持不变。
Arena 不释放内存，也不验证其他指针；请确保输入和输出 region 都处于各自分配范围内。

```js
import { readFile } from 'node:fs/promises';
import { createCKWasmArena } from './examples/wasm/host/ck-wasm-arena.mjs';

const bytes = await readFile('build/pricing_batch.wasm');
const { instance } = await WebAssembly.instantiate(bytes);
const arena = createCKWasmArena(instance);

const count = 3;
const prices = arena.copyInI64(new BigInt64Array([100n, 200n, 300n]));
const quantities = arena.copyInI64(new BigInt64Array([2n, 1n, 4n]));
const discounts = arena.copyInI64(new BigInt64Array([5n, 0n, 20n]));
const taxRates = arena.copyInI64(new BigInt64Array([100_000n, 200_000n, 50_000n]));
const totals = arena.allocI64(count);

instance.exports.pricing_batch(
  prices.ptr, quantities.ptr, discounts.ptr, taxRates.ptr, totals, count,
);
const result = arena.copyOutI64(totals, count);
```

Typed region 可使用 `allocI32`/`viewI32`、`allocU32`/`viewU32`、
`allocI64`/`viewI64`、`allocU64`/`viewU64` 或 `allocF64`/`viewF64`。
`copyIn*` 会分配空间并复制同类型 typed array；`copyOut*` 返回由 host 持有的副本。
`memory.grow` 后，之前创建的 view 会失效（也可能被 detach）；在任何可能增长 memory
的操作后重新获取 view。Arena 会在增长后刷新自己的 view 来源。

示例导出 `pricing_one` 计算一条记录，`pricing_batch` 则在一次 Wasm 调用中处理 N 条
structure-of-arrays 记录。运行 Node 聚焦检查：
`node --test examples/wasm/host/*.test.mjs`。若要比较耗时，先构建 `ckc`，然后执行：

```sh
cargo build --bin ckc
node benches/wasm/bench.mjs --ckc target/debug/ckc \
  --case pricing_one_calls --case pricing_batch_call --size 4096
```

报告会校验每个输出，区分逐条 host 调用和单次批处理调用，并记录产物字节数及 host
各阶段耗时。单一 runtime 的结果不能证明所有环境都会普遍加速。

WASM 使用 `--bounds unchecked` 与 `--overflow unchecked`。CLI 拒绝 checked
mode，不插入 implicit trap/guard；untrusted input 必须由 host 验证 offset/length。
规范 contract 见 [WASM ABI](../abi/wasm.md)。
