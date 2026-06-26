# CalcKernel

[English](README.md)

CK / CalcKernel 是一门面向高性能纯计算 kernel 的 DSL。它不是通用编程语言。
V0 会把源码文件编译成可读的 C、WAT/WASM 或 LLVM IR 输出，供 Node.js、
Python、Java、Rust、Go、C#、clang 和 WebAssembly 等宿主语言与工具链使用。

项目边界刻意保持很窄：纯计算 kernel、调用方拥有内存、无 IO、无字符串、无
runtime、无动态分配。整数 kernel 仍是主要目标；Phase 16 增加 strict `f64`，
用于数值 kernel。

当前开发快照已包含 lexer、parser、type checker、MIR、MIR validation、保守的
MIR optimization levels、C/WASM/LLVM backends、strict `f64` support、C 输出的
checked integer arithmetic、宿主语言示例、backend regression 覆盖和手动
performance suite。

## 快速开始

```sh
pnpm install
pnpm test
pnpm build
```

在源码 checkout 中，通过本地 pnpm script 运行已构建的 CLI：

```sh
pnpm ckc --help
pnpm ckc check examples/pricing.ck
pnpm ckc emit-c examples/pricing.ck --out build/pricing.c --header build/pricing.h
pnpm ckc build examples/pricing.ck --out build/libpricing
pnpm ckc build examples/pricing.ck --out build/libpricing --overflow unchecked
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
```

作为 package 安装后，`bin` 入口是 `ckc`。当前 examples、fixtures、docs 和
package smoke source 均使用 `.ck` 后缀。

## `.ck` 示例

```ck
struct Item {
  price: i64;
  qty: i64;
  discount: i64;
  tax_rate_ppm: i64;
}

export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32 {
  let i: i32 = 0;

  while i < len {
    let subtotal: i64 = items[i].price * items[i].qty;
    let after_discount: i64 = subtotal - items[i].discount;
    let tax: i64 = after_discount * items[i].tax_rate_ppm / 1000000;
    out[i] = after_discount + tax;
    i = i + 1;
  }

  return 0;
}
```

## f64 Strict Mode

Phase 16 增加第一版 strict `f64`，用于数值 kernel：

```ck
export fn axpy(a: f64, x: ptr<f64>, y: ptr<f64>, len: i32) -> f64 {
  let i: i32 = 0;
  let checksum: f64 = 0.0;

  while i < len {
    let value: f64 = a * x[i] + y[i];
    y[i] = value;
    checksum = checksum + value;
    i = i + 1;
  }

  return checksum;
}
```

支持的 f64 操作包括 `+`、`-`、`*`、`/`、unary `-`，以及 `==`、`!=`、`<`、
`<=`、`>`、`>=` comparison。C、WASM 和 LLVM backend 支持 `ptr<f64>` 和包含
`f64` field 的 struct。

Strict mode 含义：

- `f64` 映射到 C `double`、LLVM `double` 和 WASM `f64`
- JavaScript WASM interop 对 `f64` 使用 `Number`
- `f64` 是唯一 floating point type；不规划 `f32`
- 不支持 implicit int/float conversion
- 支持 exact explicit int-to-f64 cast，仅限 `i32_to_f64(x)` 和
  `u32_to_f64(x)`
- 不支持 `i64_to_f64`、`u64_to_f64` 或 f64-to-int cast
- 不支持 `f64 %`
- 不启用 fast-math flag 或 reassociation
- 不支持 SIMD
- 不做 float checked overflow
- 不支持 `NaN`、`Infinity` 或 float suffix literal 语法
- 不承诺跨 backend 浮点结果 bit-identical

`NaN`、正负 infinity 和 `-0.0` 遵循所选 backend 的普通 strict floating point
行为。它们可以由 arithmetic 产生，但 CK / CalcKernel 不提供专用 literal 语法，也不
承诺稳定的 NaN payload。测试和 benchmark 对有限 f64 结果使用 tolerance，对 NaN、
infinity 和 signed zero 分别做分类判断。

`f64` 适合 axpy、dot product、sum、scale 这类数值 kernel。金额、税费、优惠、
POS 总价和规则计算仍建议使用 `i64` fixed-point arithmetic，这样 checked integer
mode 可以明确报告 overflow 和 division error。

Explicit cast 不会打开 implicit conversion。下面两个函数是 compiler builtin，不是
runtime call：

```ck
export fn avg_i32(sum: i32, count: i32) -> f64 {
  return i32_to_f64(sum) / i32_to_f64(count);
}

export fn ratio_u32(a: u32, b: u32) -> f64 {
  return u32_to_f64(a) / u32_to_f64(b);
}
```

`i32_to_f64` 和 `u32_to_f64` 是 exact，因为所有 `i32` 和 `u32` 值都可以被 `f64`
精确表示。如果这类示例中发生除以零，结果遵循普通 strict f64 行为，可能产生
infinity 或 NaN；这不是 checked integer error。

## 生成 C

默认生成 unchecked 输出：

```sh
pnpm ckc emit-c examples/pricing.ck --out build/pricing.c --header build/pricing.h
```

显式 unchecked 写法等价：

```sh
pnpm ckc emit-c examples/pricing.ck --out build/pricing.c --header build/pricing.h --overflow unchecked
```

Checked 输出使用 checked arithmetic ABI：

```sh
pnpm ckc emit-c examples/pricing.ck --out build/pricing.checked.c --header build/pricing.checked.h --overflow checked
```

生成的 header 包含 `stdint.h`、`stdbool.h`、struct typedef 和导出函数声明。
Header 也包含用于动态库导出的 `CK_API`，以及给 C++ 消费者使用的
`extern "C"` guard。生成的 source 包含对应 header 和函数实现。Strict f64 mode 下，
C backend 将 `f64` 映射为 `double`，将 `ptr<f64>` 映射为 `double*`，并生成普通 C
double arithmetic 和 comparison。

## 构建动态库

Unchecked mode 是默认模式：

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing
```

它等价于：

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing --overflow unchecked
```

Checked arithmetic mode 会改变生成的 C ABI：函数返回 `CK_Status`，原始返回值
通过最后一个 `ck_return` 指针写出：

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
```

`CK_Status` 是 `int32_t` 状态码：

- `CK_OK`：计算成功
- `CK_ERR_OVERFLOW`：checked arithmetic overflow
- `CK_ERR_DIV_BY_ZERO`：checked 除法或取模除零
- `CK_ERR_NULL_POINTER`：生成的 checked `ck_return` 指针为 `NULL`

金额、税费、优惠和规则 kernel 需要显式报告算术失败时，使用 checked mode。
已经验证输入且最大吞吐更重要的热路径，可以使用 unchecked mode。

`build` 命令生成 C/header 文件，并用严格参数调用 clang：

```text
-std=c11 -O3 -Wall -Wextra -Werror
```

输出扩展名依平台而定：

- Linux: `.so`
- macOS: `.dylib`
- Windows: `.dll`

## 开发者 MIR 调试

开发者可以查看默认 C backend 使用的 typed MIR：

```sh
pnpm ckc emit-mir examples/pricing.ck
pnpm ckc emit-mir examples/pricing.ck --out build/pricing.mir
```

MIR 是编译器内部 IR：它带类型、基于 basic block，面向 backend 实现和调试。
它不是用户可编写的源码语言。普通用户仍应使用 `check`、`emit-c` 和 `build`。

## WASM Backend

Phase 12 增加 WASM backend，将已验证 MIR 降低为 WAT，再用捆绑的 `wabt` npm
package 编译成 WASM：

```sh
pnpm ckc emit-wat examples/scalar.ck --out build/scalar.wat
pnpm ckc emit-wasm examples/scalar.ck --out build/scalar.wasm
pnpm ckc emit-wasm examples/pricing.ck --out build/pricing.wasm
```

Phase 12 v1 ABI 目标是 `wasm32`，导出 linear memory，将 `ptr<T>` 映射为
`i32` memory offset，在 JavaScript 中用 `BigInt` 处理 `i64` / `u64` interop，
并保持 arithmetic unchecked。当前 backend 已覆盖 scalar operations、
control flow、内部函数调用、短路逻辑，以及 `pricing.ck` 这类核心
ptr/index/field load/store 模式。Phase 16 增加 f64 WASM codegen：scalar f64
parameter/return 使用 WASM `f64`，JavaScript interop 使用 `Number`，`ptr<f64>`
memory 使用 `f64.load` / `f64.store`。

WASM backend 当前只支持 unchecked mode。`emit-wat --overflow checked` 和
`emit-wasm --overflow checked` 会用清晰错误失败；需要 checked arithmetic 时请用
`emit-c` 或 `build`。Checked WASM code generation 和 bounds check 还没有实现。

ABI、struct layout、memory model、WABT assembly step 和 Node.js interop 规则见
[WASM ABI](docs/zh-CN/WASM_ABI.md)。

## LLVM Backend

Phase 13 增加 MIR-to-LLVM backend，可以生成 textual LLVM IR（`.ll`），也可以
通过 clang 构建 native dynamic library：

```text
.ck source -> CheckedProgram -> MIR -> LLVM IR text
```

```sh
pnpm ckc emit-llvm examples/pricing.ck --out build/pricing.ll
pnpm ckc build-llvm examples/pricing.ck --out build/libpricing
pnpm ckc build-llvm examples/pricing.ck --kind object --out build/pricing.o
pnpm ckc build-llvm examples/pricing.ck --out build/libpricing --target x86_64-unknown-linux-gnu
```

v1 backend 只支持 unchecked：`emit-llvm --overflow checked` 和
`build-llvm --overflow checked` 会失败，不会静默生成 unchecked LLVM IR。需要
checked arithmetic 时请使用 C backend。LLVM v1 不使用 LLVM C++ API binding、
JIT、optimizer pipeline、debug info、runtime support、allocator、bounds check 或
`slice<T>`。`build-llvm` 需要 clang；`emit-llvm` 不依赖 clang。Object output
可用于用户自己的 link 流程；static library output 暂未实现。详见
[LLVM Backend](docs/zh-CN/LLVM_BACKEND.md)。
Phase 16 增加 f64 LLVM codegen，使用 `double`、`fadd`、`fsub`、`fmul`、`fdiv`、
`fcmp`、`load double` 和 `store double`，不添加 fast-math flags。

## Node.js WASM 示例

仓库包含一个无额外依赖的 Node.js WASM 示例，通过内置 WebAssembly API 调用
`calc_items`。它不需要 native `.so`、`.dylib` 或 `.dll`。

```sh
pnpm build
pnpm ckc emit-wasm examples/pricing.ck --out build/pricing.wasm
node examples/node-wasm-call/index.mjs
```

详见 [examples/node-wasm-call](examples/node-wasm-call/README.zh-CN.md)，了解
`DataView` memory 写入、`Item` layout、pointer offset、output buffer 和
`BigInt` 映射。

## Node.js WASM f64 数组示例

对于 homogeneous `ptr<f64>` buffer，推荐在 exported WASM memory 上创建
`Float64Array` view，而不是在热路径逐元素使用 `DataView`：

```sh
pnpm build
pnpm ckc emit-wasm examples/node-wasm-f64-array/f64_array.ck --out examples/node-wasm-f64-array/f64_array.wasm
node examples/node-wasm-f64-array/index.mjs
```

Pointer ABI 不变：WASM `ptr<f64>` 仍是 `i32` byte offset，`f64` size 是 8，
`ptr<f64>[i]` 是 `base + i * 8`，`Float64Array` index 是 `byteOffset / 8`。
byte offset 必须 8-byte aligned。如果 host 调用 `memory.grow`，继续使用前要重新
创建 typed-array view。CK / CalcKernel 不提供 WASM allocator 或 runtime；memory
placement 和 buffer sizing 由 host 负责。

## Browser WASM 示例

仓库还包含一个无框架、无 bundler 的纯浏览器 WASM 示例。把 `pricing.wasm`
生成到浏览器示例目录，然后通过 HTTP server 访问：

```sh
pnpm build
pnpm ckc emit-wasm examples/pricing.ck --out examples/browser-wasm-call/pricing.wasm
cd examples/browser-wasm-call
python3 -m http.server 8000
```

打开 `http://localhost:8000/index.html`，点击 **Run pricing wasm**。

浏览器通常不能可靠地从 `file://` fetch WASM，所以请使用本地 HTTP server。
完整浏览器 memory 和 `DataView` 说明见
[examples/browser-wasm-call](examples/browser-wasm-call/README.zh-CN.md)。

## Python ctypes 示例

仓库包含一个无额外依赖的 Python 示例，通过 `ctypes` 调用生成的 pricing 动态库。

macOS/Linux：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/libpricing
python3 examples/python-ctypes-call/call_pricing.py
```

Windows 上，先生成 `pricing.dll`，再用 Python 运行同一脚本：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/pricing.dll
py examples\python-ctypes-call\call_pricing.py
```

Checked ABI 示例：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
python3 examples/python-ctypes-call/call_pricing_checked.py
```

Windows 上显式生成 `pricing_checked.dll`：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/pricing_checked.dll --overflow checked
py examples\python-ctypes-call\call_pricing_checked.py
```

详见 [examples/python-ctypes-call](examples/python-ctypes-call/README.zh-CN.md)，了解
`ctypes` struct、pointer 和 checked `CK_Status` 映射。

## Node.js FFI 示例

仓库还包含一个隔离的 Node.js FFI 示例。它的 native FFI 依赖只存在于示例目录，
不会污染根 package。

macOS/Linux：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/libpricing
cd examples/node-ffi-call
pnpm install
pnpm start
```

Windows 上，先生成 `pricing.dll`：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/pricing.dll
cd examples\node-ffi-call
pnpm install
pnpm start
```

Checked ABI 示例：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
cd examples/node-ffi-call
pnpm install
pnpm start:checked
```

Windows 上显式生成 `pricing_checked.dll`：

```sh
pnpm build
pnpm ckc build examples/pricing.ck --out build/pricing_checked.dll --overflow checked
cd examples\node-ffi-call
pnpm install
pnpm start:checked
```

详见 [examples/node-ffi-call](examples/node-ffi-call/README.zh-CN.md)，了解 Koffi
struct、pointer、`BigInt` 和 checked `CK_Status` 映射。

## Benchmarks

[bench](bench/README.zh-CN.md) 目录包含本机 pricing 和 f64 performance suite，
用于对比生成的 C、可适用时的 checked C、LLVM、WASM、JavaScript `Array`
`Number`、JavaScript `Float64Array` 和 JavaScript `BigInt` baseline，并且每个
case 都做 checksum 校验。

```sh
pnpm build
node bench/perf/run.mjs --quick
node bench/perf/run.mjs --full --save-baseline
node bench/perf/run.mjs --full --compare --threshold 10
```

Benchmark 只是粗略的本地参考，不是跨机器稳定分数。结果依赖本机硬件、Node.js、
clang、hyperfine 和当前系统负载。不要提交 `build/perf` 中的机器本地 baseline，
也不要把性能阈值放进普通 `pnpm test`。跨语言集成时，应把工作批量放进较大的
native 调用，而不是一条 item 调一次 native 函数。WASM f64 结果要按拆分路径解读：
compute-only 测 memory 已准备好后的 kernel，total 包含 input marshal 和 output
readback。JavaScript `Float64Array` 是很强的 host 热循环 baseline；如果把 host
memory movement 算进 total，WASM 不保证一定更快。当前 optimization pipeline、
本机最新 full run 摘要、baseline/compare 流程、f64 benchmark 覆盖和 backend 瓶颈见
[Performance](docs/zh-CN/PERFORMANCE.md) 和 [Optimization](docs/zh-CN/OPTIMIZATION.md)。

## 当前 V0 限制

V0 当前支持：

- `i32`、`i64`、`u32`、`u64`、`f64`、`bool`
- `ptr<T>`
- `struct`
- `fn` 和 `export fn`
- `let`、assignment、`return`、`if` / `else`、`while`
- 整数和 strict `f64` 算术与比较
- boolean 逻辑运算
- 指针索引和 struct 字段访问

V0 不支持字符串、IO、heap allocation、GC、异常、async、class、闭包、模块、
runtime library 或 JIT。WASM 和 LLVM backend 当前只支持 unchecked arithmetic。

Floating point 刻意保持很窄：CK / CalcKernel 是 f64-only。当前支持 `f64`
strict mode，不规划 `f32`。当前只实现 exact explicit `i32_to_f64` 和
`u32_to_f64` cast。implicit int/float conversion、`i64/u64` to f64 cast、
f64-to-int cast、`f64 %`、fast-math、SIMD 或 float checked overflow 都未实现。
CK / CalcKernel 不保证所有 C、LLVM、WASM 和 JavaScript target 的浮点结果
bit-identical。

V0 不做 bounds check。默认 arithmetic 是 unchecked；可选
`--overflow checked` C code generation 会检查整数 overflow 和除零，但仍不检查
pointer validity、buffer length 或 f64 overflow。WASM 和 LLVM backend 会拒绝
`--overflow checked`。调用方拥有所有输入/输出 buffer，并必须传入有效指针和长度。

## 文档

默认文档语言是英文。每份项目文档都维护对应中文译本。

English:

- [Language Specification](docs/LANGUAGE_SPEC.md)
- [Compiler Architecture](docs/COMPILER_ARCHITECTURE.md)
- [MIR](docs/MIR.md)
- [Checked Arithmetic](docs/CHECKED_ARITHMETIC.md)
- [C ABI](docs/ABI.md)
- [WASM ABI](docs/WASM_ABI.md)
- [LLVM Backend](docs/LLVM_BACKEND.md)
- [Optimization](docs/OPTIMIZATION.md)
- [Performance](docs/PERFORMANCE.md)
- [Naming Conventions](docs/NAMING_CONVENTIONS.md)
- [Migration Guide](docs/MIGRATION_IK_TO_CK.md)
- [Roadmap](docs/ROADMAP.md)
- [Release Checklist](docs/RELEASE_CHECKLIST.md)

中文：

- [语言规格](docs/zh-CN/LANGUAGE_SPEC.md)
- [编译器架构](docs/zh-CN/COMPILER_ARCHITECTURE.md)
- [MIR](docs/zh-CN/MIR.md)
- [Checked Arithmetic](docs/zh-CN/CHECKED_ARITHMETIC.md)
- [C ABI](docs/zh-CN/ABI.md)
- [WASM ABI](docs/zh-CN/WASM_ABI.md)
- [LLVM Backend](docs/zh-CN/LLVM_BACKEND.md)
- [优化](docs/zh-CN/OPTIMIZATION.md)
- [性能](docs/zh-CN/PERFORMANCE.md)
- [命名规范](docs/zh-CN/NAMING_CONVENTIONS.md)
- [迁移指南](docs/zh-CN/MIGRATION_IK_TO_CK.md)
- [路线图](docs/zh-CN/ROADMAP.md)
- [发布检查清单](docs/zh-CN/RELEASE_CHECKLIST.md)
