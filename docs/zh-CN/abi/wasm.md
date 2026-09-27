# CalcKernel 0.14 WebAssembly ABI

[English](../../abi/wasm.md)

本文档定义 `emit-wat` 与 `emit-wasm` 输出。Module export 源码中的 `export fn`，并提供一块
caller-owned linear memory；internal function 保持 internal。

WAT/WASM lowering C/Native 共用的 verified KIR。Consumer 在 KIR 构造前选择，因此 checked
mode 与 reachable Native print 在 optimization 前拒绝；backend 不增加 hidden guard，也没有
legacy optimized-MIR path。

`i32`、`u32`、`bool`、`ptr<T>` 使用 WASM `i32`；`i64`/`u64` 使用 `i64`；`f64`
使用 `f64`；void 无 `(result ...)`，void call 为 targetless。Pointer 是 little-endian linear memory 中的 byte address。
Caller 负责 allocation、validity、alignment、lifetime、growth 与 alias，并在 `memory.grow`
后重建 host view；CK 不提供 allocator。

## Target feature 与模块元数据

`--wasm-features baseline|simd128` 为 `emit-wat`、`emit-wasm` 或
`emit-kir --consumer wasm` 选择 WebAssembly target profile；默认值为 `baseline`。此值属于
规范化 KIR target profile，并改变其 SHA-256 digest。`baseline` 生成标量代码。O3 的
`simd128` 可为经过独立验证的连续 `slice<f64>`、`slice<i32>`、`slice<u32>` map、
`i32`/`u32` 到 `f64` 的转换，以及模整数求和、求积归约生成 128 位 SIMD。完整向量
使用 `f64x2` 或 `i32x4` 指令；其他不受支持的循环及 O0–O2 保持标量。

每个 WAT 和 binary module 都带有 `ck.wasm.target` custom section。其 UTF-8 payload 是确定性
JSON，schema 1，键按以下顺序排列：

```json
{"schema":1,"target":"wasm32","features":"baseline","profile_sha256":"<64 lowercase hex>"}
```

`features` 的值只能是 `baseline` 或 `simd128`。Digest 标识所选的规范化 KIR target profile；
两个 profile 也可能生成不同指令和模块字节。

`baseline` allowlist 为 WebAssembly core MVP 加 `MULTI_VALUE`，slice return ABI 需要该能力。
`simd128` allowlist 再加 `SIMD128`。Backend 拒绝 `RELAXED_SIMD`、threads、Memory64
以及所有未声明的 proposal；选择 profile 不会放宽 instruction 验证。Host 应读取并验证
`ck.wasm.target`，再确认 runtime 支持其中声明的 profile，然后实例化 module。使用标准
WebAssembly API 的 host 可以先 compile、检查 `WebAssembly.Module.customSections`，再于实例化前完成检查。

完整宽度的 SIMD 内存操作使用连续 16 字节 load/store，以及已证明的自然对齐（`f64`
为 8 字节，`i32`/`u32` 为 4 字节）。两 lane 的 `i32`/`u32` 到 `f64x2` 转换通过
`v128.load64_zero` 精确读取 8 个源字节，转换结果存储 16 字节。支持逐 lane splat、
加、减、乘、取负和纯比较/选择；`f64x2` 还支持除法。整数归约先按模运算折叠四个 lane，
再更新传递的标量累加器。向量循环只处理完整 chunk，余数或短 trip 交给原有标量循环。
简单的未知别名循环使用不会陷阱的 Wasm32 地址范围与不相交谓词；谓词失败时走标量路径。
每个浮点 lane 保留原标量求值顺序及舍入，不引入 relaxed SIMD 或融合运算；整数运算按
2^32 取模。不受支持的候选保持标量。源码中的 `noalias` contract 仍由 caller 负责。

## 标量地址 lowering

O3 可以对范围有限、经过独立检查的直接指针循环使用 Wasm32 字节地址游标：循环中只有一个
模整数归纳变量改变，且直接访问 4 或 8 字节的基本类型。游标在原有回边递增；不支持的循环保留
通常的地址计算。`baseline` 与 `simd128` profile 都可使用这一路标量 lowering。

对于结构字段访存，CLI 发射仅在已验证的对齐事实证明原有 32 位地址加法不会回绕时，
才把常量字段位移折入 load 的 memarg；否则保留显式加法。memarg 对齐不得超过
字段的实际对齐，也不证明指针有效、无别名或增加运行时 guard。单独 KIR module 的直接
发射没有已验证的 contract 上下文，因而保留显式加法。

`slice<T>` parameter 是 data,length 顺序的两个 collision-safe `i32`。Stored descriptor 为
8 bytes、alignment 4，offset 0 为 address，offset 4 为 `u32` length。Internal call 与
multi-value return 保持同一顺序。

WASM 只接受 `--overflow unchecked` 与 `--bounds unchecked`；任一 checked selection 在
输出前 rejected，不插入隐式 slice guard 或 trap。C/Native checked status ABI 不属于本 ABI。

WebAssembly 0.14 没有 runtime print；export root 可达的 print 被拒绝。Internal `main`
不会创建 WASI 或 browser entry。
