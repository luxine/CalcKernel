# IK / IntKernel WASM ABI

[English](../WASM_ABI.md)

本文档定义 IntKernel Phase 12 的 WebAssembly ABI。当前 Phase 12 实现可以为
unchecked scalar 运算、control flow、内部函数调用、逻辑短路，以及核心
`ptr<T>` memory load/store 模式生成 WAT 和 WASM。

## Phase 12 目标

Phase 12 在 MIR 之后增加 WASM backend：

```text
.ik source
  -> lexer
  -> parser
  -> type checker
  -> CheckedProgram / Typed Program
  -> MIR lowering
  -> MIR validator
  -> MIR-to-WAT backend
  -> .wat
  -> WAT-to-WASM assembly through the `wabt` npm package
  -> .wasm
```

当前 CLI 命令：

```sh
ikc emit-wat examples/scalar.ik --out build/scalar.wat
ikc emit-wat examples/scalar.ik
ikc emit-wasm examples/scalar.ik --out build/scalar.wasm
ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
```

`emit-wat` 可以写文件，也可以输出到 stdout。`emit-wasm` 输出二进制文件，因此必须
提供 `--out`。

Phase 12 v1 backend 目标是 `wasm32`，输入是已验证的 MIR，并保持与 C backend
一致的 no-runtime 和 caller-owned-memory 模型。

`emit-wasm` 使用作为 IntKernel runtime dependency 捆绑的 `wabt` npm package
编译生成的 WAT。发布后的 CLI 不需要外部 `wat2wasm` 可执行文件。

## WASM v1 范围

Phase 12 v1 设计支持：

- `i32`
- `i64`
- `u32`
- `u64`
- `f64`
- `bool`
- `ptr<T>`
- 确定性的 struct memory layout
- exported functions
- non-exported internal functions
- exported linear memory
- unchecked arithmetic
- `f64` arithmetic、comparison、load 和 store
- exact explicit `i32_to_f64` / `u32_to_f64` cast
- 通过 `wabt` 完成 WAT-to-WASM assembly

WASM floating point 遵循项目级 f64-only policy：`f64` 是唯一 floating point
type，不规划 `f32`，不支持 implicit int/float conversion，当前只支持 exact
explicit `i32_to_f64` / `u32_to_f64` cast。scalar f64 interop 使用 JavaScript
`Number`。

Phase 12 v1 暂不支持：

- checked overflow
- WASI
- imports
- heap allocation
- strings
- bounds check
- `slice<T>`
- runtime library
- SIMD
- threads
- GC
- exceptions

当前 `emit-wasm` 实现支持 `ptr<T>` load/store codegen，例如 `items[i].price`
和 `out[i] = value` 这类 MIR place。它仍然不添加 bounds check 或 pointer
validity check。

Phase 16.9 支持 `f64` scalar arithmetic、comparison、load 和 store codegen。
`f64` 的 size 是 8、alignment 是 8，scalar ABI type 是 `f64`，宿主 JavaScript
interop 使用 `Number`，不是 `BigInt`。

## 类型映射

| IntKernel type | WASM value type |
| --- | --- |
| `i32` | `i32` |
| `u32` | `i32` |
| `bool` | `i32` |
| `i64` | `i64` |
| `u64` | `i64` |
| `f64` | `f64` |
| `ptr<T>` | `i32` memory offset |

WASM 没有独立的 `u32` 或 `u64` value type。Signedness 通过 division、
remainder 和 comparison 指令选择体现。

`bool` 使用 `i32`：`0` 表示 false，任何非零值表示 true。Codegen 应该为
IntKernel boolean 表达式生成规范化的 `0` 或 `1`。

JavaScript 的 WebAssembly API 用 `BigInt` 表示 `i64` 和 `u64` 参数与返回值，
用 JavaScript `Number` 表示 `f64` 参数与返回值。

f64 语义刻意保持为 WebAssembly 的普通 strict 行为：

- arithmetic 使用 `f64.add`、`f64.sub`、`f64.mul`、`f64.div` 和 `f64.neg`
- comparison 使用 `f64.eq`、`f64.ne`、`f64.lt`、`f64.le`、`f64.gt` 和
  `f64.ge`
- memory 使用 `f64.load` 和 `f64.store`
- NaN、infinity 和 `-0.0` 遵循 WebAssembly f64 行为
- host 测试应使用 `Number.isNaN`、带符号 infinity 判断、`Object.is` 判断 `-0`，
  并对有限值使用 tolerance，而不是 bit equality
- `f64` 绝不使用 `BigInt` 传递
- `i32_to_f64(x)` lowering 为 `f64.convert_i32_s`
- `u32_to_f64(x)` lowering 为 `f64.convert_i32_u`
- `i64_to_f64`、`u64_to_f64`、f64-to-int cast 和 implicit conversion 仍不支持

## Function ABI

导出的 IntKernel 函数以源码中的名称从 WASM module 导出。

IntKernel 示例：

```ik
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

目标 WAT 形态：

```wat
(func $add_i64 (export "add_i64")
  (param $a i64)
  (param $b i64)
  (result i64)
  ;; implementation
)
```

Boolean return 使用 `i32`：

```ik
export fn is_positive(a: i64) -> bool {
  return a > 0;
}
```

目标 WASM result：

```wat
(result i32)
```

非导出的 IntKernel 函数生成为内部 WASM function，不生成 export entry。

## Pointer ABI

`ptr<T>` 是 `wasm32` linear-memory offset，用 `i32` 表示。`ptr<f64>` 仍然是
`i32` byte offset；indexing 每个元素前进 8 bytes。

IntKernel 示例：

```ik
export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
  ...
}
```

目标 WASM ABI：

```wat
(param $items i32)
(param $len i32)
(param $out i32)
(result i32)
```

调用方负责：

- 将 `Item` 数组写入 WASM memory
- 传入 `items` memory offset
- 传入 `len`
- 在 WASM memory 中预留 output buffer
- 传入 `out` memory offset
- 调用后读取 output buffer

编译器不拥有、不分配、不增长、也不验证这些 buffer。

## Memory

Phase 12 v1 生成一个 exported memory：

```wat
(memory (export "memory") 1)
```

一个 WebAssembly page 是 64 KiB。

Phase 12 v1 不提供 allocator。宿主程序手动选择 linear memory offset。Backend
不生成 `memory.grow` helper。未来阶段可以加入 simple allocator，但 Phase 12
不做。

## Struct Layout

WASM 使用 IntKernel 定义的确定性布局，不依赖宿主 C 编译器。这不会改变 C ABI
layout：生成的 C header 和 C harness 仍使用目标 C 编译器的正常 struct layout
规则。

Primitive layout：

| Type | Size | Alignment |
| --- | ---: | ---: |
| `i32` | 4 | 4 |
| `u32` | 4 | 4 |
| `bool` | 4 | 4 |
| `ptr<T>` | 4 | 4 |
| `i64` | 8 | 8 |
| `u64` | 8 | 8 |
| `f64` | 8 | 8 |

Struct layout 规则：

- field 按声明顺序布局
- 每个 field offset 按该 field alignment 对齐
- 需要时插入 padding
- struct alignment 是所有 field alignment 的最大值
- struct size padding 到 struct alignment

示例：

```ik
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}
```

Layout：

| Field | Offset |
| --- | ---: |
| `price` | 0 |
| `qty` | 8 |
| `discount` | 16 |
| `tax_rate_ppm` | 24 |

`sizeof(Item) = 32`

`align(Item) = 8`

## Load 和 Store 映射

`items[i].price` 降低为地址计算和 load：

```text
address = items + i * sizeof(Item) + offset(price)
i64.load address
```

`out[i] = value` 降低为地址计算和 store：

```text
address = out + i * sizeof(i64)
i64.store address value
```

Backend 根据 value type 选择 load/store 指令：

- `i32.load` / `i32.store` 用于 `i32`、`u32`、`bool` 和 `ptr<T>`
- `i64.load` / `i64.store` 用于 `i64` 和 `u64`
- `f64.load` / `f64.store` 用于 `f64`

所有宿主侧示例都应该使用 little-endian 读写。WebAssembly linear memory 是
little-endian。宿主测试和 harness 应对 `f64` buffer 使用
`DataView.getFloat64(offset, true)` 和 `DataView.setFloat64(offset, value,
true)`。

## Arithmetic 映射

Phase 12 v1 是 unchecked。

加法、减法和乘法在同一宽度下 signed/unsigned 使用相同 WASM 算术指令：

- `i32.add`、`i32.sub`、`i32.mul`
- `i64.add`、`i64.sub`、`i64.mul`
- `f64.add`、`f64.sub`、`f64.mul`

除法和取模必须选择 signed 或 unsigned 指令：

- `i32.div_s` / `i32.div_u`
- `i64.div_s` / `i64.div_u`
- `i32.rem_s` / `i32.rem_u`
- `i64.rem_s` / `i64.rem_u`

F64 division 使用 `f64.div`。F64 remainder 不支持。Unary `-f64` 降低为
`f64.neg`；integer negation 继续使用现有 zero-subtraction lowering。

比较也必须选择 signed 或 unsigned 指令：

- `i32.lt_s` / `i32.lt_u`
- `i32.le_s` / `i32.le_u`
- `i32.gt_s` / `i32.gt_u`
- `i32.ge_s` / `i32.ge_u`
- `i64.lt_s` / `i64.lt_u`
- `i64.le_s` / `i64.le_u`
- `i64.gt_s` / `i64.gt_u`
- `i64.ge_s` / `i64.ge_u`

相等比较不区分 signedness：

- `i32.eq`、`i32.ne`
- `i64.eq`、`i64.ne`

F64 comparison 使用标准 WASM f64 predicates：

- `f64.eq`、`f64.ne`
- `f64.lt`、`f64.le`
- `f64.gt`、`f64.ge`

Phase 20 explicit int-to-f64 cast 使用标准 WASM conversion opcode：

- `i32_to_f64(x)` 使用 `f64.convert_i32_s`
- `u32_to_f64(x)` 使用 `f64.convert_i32_u`

## Checked Overflow

Phase 12 v1 不支持 checked WASM code generation。本阶段 WASM backend 只支持
unchecked mode；遇到 `--overflow checked` 必须拒绝，而不能静默生成 unchecked
output。

如果用户执行：

```sh
ikc emit-wat input.ik --overflow checked
ikc emit-wasm input.ik --overflow checked
```

编译器必须报告：

```text
error: WASM backend does not support --overflow checked yet.
help: use --overflow unchecked, or use emit-c/build for checked C output.
```

需要 checked arithmetic 时，请使用 C backend（`emit-c` 或 `build`）。WASM
checked arithmetic 需要为 WASM 指令显式 lowering overflow check，属于未来工作。

## Node.js Interop

Node.js 可以用内置 WebAssembly API 实例化生成的 WASM：

```js
const bytes = await fs.promises.readFile("build/pricing.wasm");
const { instance } = await WebAssembly.instantiate(bytes);
```

Interop 规则：

- 使用 `instance.exports.memory` 访问 linear memory
- 使用 `DataView` 或 typed arrays 写入 input buffer、读取 output buffer
- 使用 little-endian `DataView` 方法
- 将 `ptr<T>` 作为 numeric memory offset 传入
- `i64` / `u64` 参数和返回值使用 `BigInt`
- `f64` 参数和返回值使用 JavaScript `Number`
- 将 `bool` result 解释为 `result !== 0`

宿主程序负责为输入和输出 buffer 选择不重叠的 memory region。

上面 `Item` layout 的宿主侧写入 helper 示例：

```js
const memory = instance.exports.memory;
const view = new DataView(memory.buffer);
const ITEM_SIZE = 32;

function writeItem(offset, item) {
  view.setBigInt64(offset + 0, item.price, true);
  view.setBigInt64(offset + 8, item.qty, true);
  view.setBigInt64(offset + 16, item.discount, true);
  view.setBigInt64(offset + 24, item.taxRatePpm, true);
}

writeItem(0, {
  price: 1234n,
  qty: 2n,
  discount: 0n,
  taxRatePpm: 100000n
});

// ptr<Item> 参数就是数字 offset。
const price = instance.exports.first_price(0);
```

对于 `ptr<f64>` buffer，`DataView` 适合 byte-level ABI 测试和 mixed-width
layout 检查：

```js
view.setFloat64(valuesOffset + 8, 2.5, true);
const value = view.getFloat64(valuesOffset + 8, true);
```

对于大块 homogeneous `f64` 数组，推荐使用指向 exported WASM memory 的
`Float64Array` view：

```js
const memory = instance.exports.memory;
const values = new Float64Array(memory.buffer);
const xOffset = 0;
const yOffset = 64;
const xIndex = xOffset / 8;
const yIndex = yOffset / 8;

values.set([1.0, 2.0, 3.0, 4.0], xIndex);
values.set([0.5, 1.25, 1.25, 2.0], yIndex);

const checksum = instance.exports.axpy_f64(1.25, xOffset, yOffset, 4);
const y = values.subarray(yIndex, yIndex + 4);
```

同样的 pointer 规则仍然适用：

- WASM `ptr<f64>` 是 `i32` byte offset。
- `f64` size 是 8 bytes。
- `ptr<f64>[i]` 使用 byte offset `base + i * 8`。
- `Float64Array` index 是 `byteOffset / 8`。
- `byteOffset` 必须 8-byte aligned。

如果 host 调用了 `memory.grow`，旧的 `Float64Array` view 可能失效。请在 grow
之后创建 view，并在后续任何 grow 后重新创建 view。IK 不提供 allocator 或 runtime；
memory placement 和 buffer sizing 仍由 host 负责。这是 low-copy host pattern，
不是所有输入来源都 zero-copy 的承诺：如果数据原本不在 WASM memory 中，host 仍需
承担放置输入的成本。`DataView` 仍适合 byte-exact ABI test、mixed-width struct 和
layout debugging。完整 Node.js 示例见 `examples/node-wasm-f64-array/`。

## Browser Interop

生成的 WASM 也可以通过标准 WebAssembly API 在浏览器中运行。
`examples/browser-wasm-call/` 中的浏览器示例使用：

- `fetch("./pricing.wasm")`
- 可用时使用 `WebAssembly.instantiateStreaming`
- fallback 到 `arrayBuffer` + `WebAssembly.instantiate`
- 用 `DataView` 做 little-endian memory 读写
- 用 numeric memory offset 表示 `ptr<T>`
- JS/WASM 边界上的 `i64` / `u64` 使用 `BigInt`
- JS/WASM 边界上的 `f64` 使用 `Number`

浏览器通常不能从 `file://` fetch `.wasm`。请通过本地 HTTP server 运行示例：

```sh
pnpm ikc emit-wasm examples/pricing.ik --out examples/browser-wasm-call/pricing.wasm
cd examples/browser-wasm-call
python3 -m http.server 8000
```

然后打开 `http://localhost:8000/index.html`。

## Benchmark Notes

`bench/wasm_pricing_benchmark.mjs` 中的 WASM benchmark 使用和 JavaScript/C
benchmark harness 相同的 batched `pricing.ik` workload shape。先生成 module：

```sh
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm --overflow unchecked
node bench/wasm_pricing_benchmark.mjs
```

Benchmark 会将大型 `Item` array 写入 exported memory，调用
`calc_items(itemsOffset, len, outOffset)`，再用 `DataView` 读取 output buffer。
大输入可能需要在 host 侧调用 `memory.grow`。这只是 benchmark setup code；
IntKernel V0 不提供 allocator 或 runtime memory-growth helper。

Benchmark 结果只是粗略本地参考，不证明 checked arithmetic safety。Phase 12 WASM
backend 只支持 unchecked。

## Phase 12 不做 Bounds Check

Phase 12 v1 不添加 bounds check。

原因：

- `ptr<T>` 不携带长度
- 编译器不知道 `out` buffer 的长度
- 编译器无法验证任意宿主传入的 memory offset
- 编译器不验证 pointer offset 或 pointer lifetime

调用方责任：

- 传入有效 memory offset
- 当函数读取 `items[i]` 时，确保 `items` 至少指向 `len` 个 `Item`
- 当函数写入 `out[i]` 时，确保 `out` 指向足够可写内存
- 除非具体 kernel 已设计为允许 aliasing，否则避免 input/output buffer 重叠

未来 bounds check 应等待携带长度的类型，例如 `slice<T>` 或显式
pointer-plus-length metadata。
