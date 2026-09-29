# CalcKernel 0.15 WebAssembly ABI

[English](../../abi/wasm.md)

本文档定义 `emit-wat` 与 `emit-wasm` 输出。Module export 源码中的 `export fn`，并提供一块
caller-owned linear memory；internal function 保持 internal。

WAT/WASM lowering C/Native 共用的 verified KIR。Consumer 在 KIR 构造前选择，因此 checked
mode 与 reachable Native print 在 optimization 前拒绝。Target lowering 不添加未经验证的安全检查；
O3 可包含显式、经过验证的优化谓词，用于选择快路径并保留源码标量路径作为 fallback。不存在
legacy optimized-MIR path。

`i32`、`u32`、`bool`、`ptr<T>` 使用 WASM `i32`；`i64`/`u64` 使用 `i64`；`f64`
使用 `f64`；void 无 `(result ...)`，void call 为 targetless。Pointer 是 little-endian linear memory 中的 byte address。
Caller 负责 allocation、validity、alignment、lifetime、growth 与 alias，并在 `memory.grow`
后重建 host view；CK 不提供 allocator。

## Target feature 与模块元数据

`--wasm-features baseline|simd128` 为 `emit-wat`、`emit-wasm` 或
`emit-kir --consumer wasm` 选择 WebAssembly target profile；默认值为 `baseline`。此值属于
规范化 KIR target profile，并改变其 SHA-256 digest。`baseline` 不生成 SIMD 指令；两个
profile 都允许 Bulk Memory 指令。O3 的 `simd128` 可为经过独立验证的连续 `slice<f64>`、
`slice<i32>`、`slice<u32>` map、受支持的 `i32`/`u32` 到 `f64` 转换、模整数求和/求积归约，
以及几种封闭源码形态生成 128 位 SIMD：`f64x2` 仿射 map（包括经过测试的嵌套矩阵列循环）、
受限的分段存储树、经过证明的 normalization 快路径，以及边界剥离后的九次加载 `3x3` stencil
内部区域。这些是特定形态的能力，不表示一般矩阵、stencil 或分支都可向量化。完整向量使用
`f64x2` 或 `i32x4` 指令；其他不受支持的 SIMD 候选及 O0–O2 保持标量。

每个 WAT 和 binary module 都带有 `ck.wasm.target` custom section。其 UTF-8 payload 是确定性
JSON，schema 2，键按以下顺序排列：

```json
{"schema":2,"target":"wasm32","features":"baseline","profile_sha256":"<64 lowercase hex>"}
```

`features` 的值只能是 `baseline` 或 `simd128`。Digest 标识所选的规范化 KIR target profile。
0.15 的 capability encoding 会改变两个 Wasm profile 的 digest；非 Wasm profile digest 保持不变。
两个 profile 也可能生成不同指令和模块字节。

`baseline` allowlist 为 WebAssembly core MVP、`MULTI_VALUE`（slice return ABI 需要）以及
`BULK_MEMORY`。`simd128` allowlist 在此基础上增加 `SIMD128`。Backend 拒绝
`RELAXED_SIMD`、threads、Memory64 以及所有未声明的 proposal；选择 profile 不会放宽
instruction 验证。Host 应读取并验证 `ck.wasm.target`，再确认 runtime 支持该 profile 及其
0.15 capability set，然后实例化 module。使用标准 WebAssembly API 的 host 可以先 compile、检查
`WebAssembly.Module.customSections`，再于实例化前完成检查。

O3 下，符合条件的直接指针 `i32`/`u32` 复制循环和重复字节填充可能使用带 guard 的
Bulk Memory 快速路径。Guard 会证明完整地址范围位于当前 Wasm memory 中且 32 位地址不回绕；
复制还要求源、目标范围不相交。证明失败时执行原标量循环，以保留重叠时的行为及后续 trap
之前已发生的写入。O0 保持这些循环为标量。

完整宽度的 SIMD 内存操作使用连续 16 字节 load/store，以及已证明的自然对齐（`f64`
为 8 字节，`i32`/`u32` 为 4 字节）。两 lane 的 `i32`/`u32` 到 `f64x2` 转换通过
`v128.load64_zero` 精确读取 8 个源字节，转换结果存储 16 字节。支持逐 lane splat、
加、减、乘、取负和纯比较/选择；`f64x2` 还支持除法。符合条件的模运算 `i32`/`u32`
求和跨 chunk 传递 `i32x4` 累加器，只在退出向量循环时折叠 lane，并将原标量初值计入
一次。模运算求积仍在每个四 lane chunk 后折叠，再更新标量累加器。向量循环只处理完整
chunk，余数或短 trip 交给原有标量循环。
未知别名的受支持循环使用不会陷阱的 Wasm32 地址范围与不相交谓词；谓词失败时走标量路径。
O3 仿射直接 map 与受支持的矩阵列循环使用 VF2，并由成本模型选择 UF1/2/4；封闭分段树使用
VF2、UF1 或 UF4。通用 map、转换、归约和 stencil 内部循环使用 UF1。严格浮点源码表达式仍发射
独立的乘与加，不会收缩为 FMA 或重结合。每个浮点 lane 保留原标量求值顺序及舍入，不引入
relaxed SIMD；整数运算按 2^32 取模。不受支持的候选保持标量。源码中的 `noalias` contract 仍由 caller 负责。

内部 `WasmSliceRange` predicate 使用扩宽后的无符号 64 位运算。`count == 0` 时结果为 true；否则
要求 `start + count <= slice.len`，并要求 exclusive end byte address
`slice.data + (start + count) * element_bytes` 同时不超过 `2^32` 和当前的
`memory.size * 65536`。它仅适用于 Wasm32 SIMD128 KIR，`start`/`count` 必须是 `u32`，slice 元素
仅限 `i32`、`u32`、`i64`、`u64` 或 `f64`，且宽度必须匹配。该内部 total range predicate 是优化谓词，
不会让一般内存访问进入 checked mode。每种受支持 SIMD plan 的独立 checker 都会重建精确标量访问范围及
适用的别名证据，并验证所需的原标量、边界或源码控制流路径。当前用途包括受支持的仿射 map/矩阵列、
封闭分段树和规范化 stencil 内部区域；任意手工构造的谓词都不能为内存访问授权。

## 标量地址 lowering

O3 可以对范围有限、经过独立验证的直接指针循环使用 Wasm32 字节地址游标：循环中只有一个
模整数归纳变量改变，且直接访问 4 或 8 字节的基本类型。游标在原有回边递增；不支持的循环保留
通常的地址计算。`baseline` 与 `simd128` profile 都可使用这一路标量 lowering。此编译期放置证明保留
源码的 32 位地址运算；它不是运行时边界、指针有效性或别名检查。

对于结构字段访存，CLI 发射仅在已验证的对齐事实证明原有 32 位地址加法不会回绕时，
才把常量字段位移折入 load 的 memarg；否则保留显式加法。memarg 对齐不得超过
字段的实际对齐，也不证明指针有效、无别名或增加运行时 guard。单独 KIR module 的直接
发射没有已验证的 contract 上下文，因而保留显式加法。

`slice<T>` parameter 是 data,length 顺序的两个 collision-safe `i32`。Stored descriptor 为
8 bytes、alignment 4，offset 0 为 address，offset 4 为 `u32` length。Internal call 与
multi-value return 保持同一顺序。

WASM 只接受 `--overflow unchecked` 与 `--bounds unchecked`；任一 checked selection 在输出前 rejected。
不提供 checked 安全 mode，也不插入隐式 checked slice trap。O3 优化谓词可守护专用快路径；谓词失败时执行
原源码路径，保留其正常 WebAssembly trap 行为。C/Native checked status ABI 不属于本 ABI。

WebAssembly 0.15 没有 runtime print；export root 可达的 print 被拒绝。Internal `main`
不会创建 WASI 或 browser entry。
