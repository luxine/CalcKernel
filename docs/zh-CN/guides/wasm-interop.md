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

使用 Node 直接构建并运行完整 host 示例（runtime 已内置 WebAssembly 支持）：

```sh
cargo build --release --locked --bin ckc
target/release/ckc emit-wasm examples/wasm/pricing_batch.ck \
  --out build/pricing_batch.wasm --opt-level 3 \
  --wasm-features baseline --overflow unchecked --bounds unchecked
node examples/wasm/host/pricing-batch.mjs build/pricing_batch.wasm
```

示例只分配一次四个输入列和一个输出列，主动增长一次 memory 以演示 view 失效，随后从当前
memory buffer 重新获取所有 view，再通过同一 arena 更新并提交三个 batch。它会将三组结果打印为
JSON。应用中可在热循环开始前按需 reserve 预期工作区；不要在可能执行 `memory.grow` 后继续使用
增长前创建的 typed array。

Typed region 可使用 `allocI32`/`viewI32`、`allocU32`/`viewU32`、
`allocI64`/`viewI64`、`allocU64`/`viewU64` 或 `allocF64`/`viewF64`。
`copyIn*` 会分配空间并复制同类型 typed array；`copyOut*` 返回由 host 持有的副本。
`memory.grow` 后，之前创建的 view 会失效（也可能被 detach）；在任何可能增长 memory
的操作后重新获取 view。Arena 会在增长后刷新自己的 view 来源。

示例导出 `pricing_one` 计算一条记录，`pricing_batch` 则在一次 Wasm 调用中处理 N 条
structure-of-arrays 记录。相同目录还包含 growth、对齐、非法长度和批量复用测试。运行 Node 聚焦检查：
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

Unchecked memory contract 与浮点语义相互独立：默认 `f64` 运算保留 strict 源码求值顺序和
binary64 舍入。`baseline` 是默认 feature profile，不发射 SIMD；显式选择
`--wasm-features simd128` 才启用 ABI 中列出的、经过验证的 SIMD 形态，也不会启用 Relaxed SIMD、
FMA、fast math 或重结合。若源代码声明 `requires noalias(a, b)`，这是调用者必须满足的前置条件：
即使模块接受任意 pointer 值，也必须传入符合声明长度且互不重叠的 range。

性能结论要区分 kernel 调用、host 准备/读取，以及首次编译/实例化。仓库的 Node/V8 runner 测量
CK WASM 并校验输出，没有 Clang/Rust WASM oracle 耗时。Native 或历史 Native performance report
不能证明相同目标下的 WASM 追平。记录的首次模块编译时间不是浏览器冷启动；每轮端到端测量使用
已实例化模块，且排除 memory growth。比较实现时应保留这些边界。
