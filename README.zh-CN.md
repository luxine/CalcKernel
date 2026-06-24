# IntKernel

[English](README.md)

IntKernel 是一门小型整数计算 DSL 编译器。它不是通用编程语言。V0 将 `.ik`
源码编译成可读的 C 源文件和头文件，再由宿主平台的 C 编译器编译成动态库，
供 Node.js、Python、Java、Rust、Go、C# 等语言调用。

项目边界刻意保持很窄：纯整数 kernel、调用方拥有内存、无 runtime、无动态
分配。

V0.1 已包含 C/C++ header ABI 加固、动态库符号导出、struct layout 验证、
Python `ctypes` 集成、Node.js FFI 示例和一个小型 benchmark harness。

## 快速开始

```sh
pnpm install
pnpm test
pnpm build
```

在源码 checkout 中，通过本地 pnpm script 运行已构建的 CLI：

```sh
pnpm ikc --help
pnpm ikc check examples/pricing.ik
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h
pnpm ikc build examples/pricing.ik --out build/libpricing
pnpm ikc build examples/pricing.ik --out build/libpricing --overflow unchecked
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
```

作为 package 安装后，`bin` 入口是 `ikc`。

## `.ik` 示例

```ik
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

## 生成 C

默认生成 unchecked 输出：

```sh
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h
```

显式 unchecked 写法等价：

```sh
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h --overflow unchecked
```

Checked 输出使用 checked arithmetic ABI：

```sh
pnpm ikc emit-c examples/pricing.ik --out build/pricing.checked.c --header build/pricing.checked.h --overflow checked
```

生成的 header 包含 `stdint.h`、`stdbool.h`、struct typedef 和导出函数声明。
Header 也包含用于动态库导出的 `IK_API`，以及给 C++ 消费者使用的
`extern "C"` guard。生成的 source 包含对应 header 和函数实现。

## 构建动态库

Unchecked mode 是默认模式：

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing
```

它等价于：

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing --overflow unchecked
```

Checked arithmetic mode 会改变生成的 C ABI：函数返回 `IK_Status`，原始返回值
通过最后一个 `ik_return` 指针写出：

```sh
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
```

`IK_Status` 是 `int32_t` 状态码：

- `IK_OK`：计算成功
- `IK_ERR_OVERFLOW`：checked arithmetic overflow
- `IK_ERR_DIV_BY_ZERO`：checked 除法或取模除零
- `IK_ERR_NULL_POINTER`：生成的 checked `ik_return` 指针为 `NULL`

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
pnpm ikc emit-mir examples/pricing.ik
pnpm ikc emit-mir examples/pricing.ik --out build/pricing.mir
```

MIR 是编译器内部 IR：它带类型、基于 basic block，面向 backend 实现和调试。
它不是用户可编写的源码语言。普通用户仍应使用 `check`、`emit-c` 和 `build`。

## WASM Backend

Phase 12 增加 WASM backend，将已验证 MIR 降低为 WAT，再用捆绑的 `wabt` npm
package 编译成 WASM：

```sh
pnpm ikc emit-wat examples/scalar.ik --out build/scalar.wat
pnpm ikc emit-wasm examples/scalar.ik --out build/scalar.wasm
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
```

Phase 12 v1 ABI 目标是 `wasm32`，导出 linear memory，将 `ptr<T>` 映射为
`i32` memory offset，在 JavaScript 中用 `BigInt` 处理 `i64` / `u64` interop，
并保持 arithmetic unchecked。当前 backend 已覆盖 scalar operations、
control flow、内部函数调用、短路逻辑，以及 `pricing.ik` 这类核心
ptr/index/field load/store 模式。

WASM backend 当前只支持 unchecked mode。`emit-wat --overflow checked` 和
`emit-wasm --overflow checked` 会用清晰错误失败；需要 checked arithmetic 时请用
`emit-c` 或 `build`。Checked WASM code generation 和 bounds check 还没有实现。

ABI、struct layout、memory model、WABT assembly step 和 Node.js interop 规则见
[WASM ABI](docs/zh-CN/WASM_ABI.md)。

## LLVM Backend

Phase 13 增加 MIR-to-LLVM backend，可以生成 textual LLVM IR（`.ll`），也可以
通过 clang 构建 native dynamic library：

```text
.ik / .ik source -> CheckedProgram -> MIR -> LLVM IR text
```

```sh
pnpm ikc emit-llvm examples/pricing.ik --out build/pricing.ll
pnpm ikc build-llvm examples/pricing.ik --out build/libpricing
pnpm ikc build-llvm examples/pricing.ik --kind object --out build/pricing.o
pnpm ikc build-llvm examples/pricing.ik --out build/libpricing --target x86_64-unknown-linux-gnu
```

v1 backend 只支持 unchecked：`emit-llvm --overflow checked` 和
`build-llvm --overflow checked` 会失败，不会静默生成 unchecked LLVM IR。需要
checked arithmetic 时请使用 C backend。LLVM v1 不使用 LLVM C++ API binding、
JIT、optimizer pipeline、debug info、runtime support、allocator、bounds check 或
`slice<T>`。`build-llvm` 需要 clang；`emit-llvm` 不依赖 clang。Object output
可用于用户自己的 link 流程；static library output 暂未实现。详见
[LLVM Backend](docs/zh-CN/LLVM_BACKEND.md)。

## Node.js WASM 示例

仓库包含一个无额外依赖的 Node.js WASM 示例，通过内置 WebAssembly API 调用
`calc_items`。它不需要 native `.so`、`.dylib` 或 `.dll`。

```sh
pnpm build
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
node examples/node-wasm-call/index.mjs
```

详见 [examples/node-wasm-call](examples/node-wasm-call/README.zh-CN.md)，了解
`DataView` memory 写入、`Item` layout、pointer offset、output buffer 和
`BigInt` 映射。

## Browser WASM 示例

仓库还包含一个无框架、无 bundler 的纯浏览器 WASM 示例。把 `pricing.wasm`
生成到浏览器示例目录，然后通过 HTTP server 访问：

```sh
pnpm build
pnpm ikc emit-wasm examples/pricing.ik --out examples/browser-wasm-call/pricing.wasm
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
pnpm ikc build examples/pricing.ik --out build/libpricing
python3 examples/python-ctypes-call/call_pricing.py
```

Windows 上，先生成 `pricing.dll`，再用 Python 运行同一脚本：

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing.dll
py examples\python-ctypes-call\call_pricing.py
```

Checked ABI 示例：

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
python3 examples/python-ctypes-call/call_pricing_checked.py
```

Windows 上显式生成 `pricing_checked.dll`：

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing_checked.dll --overflow checked
py examples\python-ctypes-call\call_pricing_checked.py
```

详见 [examples/python-ctypes-call](examples/python-ctypes-call/README.zh-CN.md)，了解
`ctypes` struct、pointer 和 checked `IK_Status` 映射。

## Node.js FFI 示例

仓库还包含一个隔离的 Node.js FFI 示例。它的 native FFI 依赖只存在于示例目录，
不会污染根 package。

macOS/Linux：

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/libpricing
cd examples/node-ffi-call
pnpm install
pnpm start
```

Windows 上，先生成 `pricing.dll`：

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing.dll
cd examples\node-ffi-call
pnpm install
pnpm start
```

Checked ABI 示例：

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/libpricing_checked --overflow checked
cd examples/node-ffi-call
pnpm install
pnpm start:checked
```

Windows 上显式生成 `pricing_checked.dll`：

```sh
pnpm build
pnpm ikc build examples/pricing.ik --out build/pricing_checked.dll --overflow checked
cd examples\node-ffi-call
pnpm install
pnpm start:checked
```

详见 [examples/node-ffi-call](examples/node-ffi-call/README.zh-CN.md)，了解 Koffi
struct、pointer、`BigInt` 和 checked `IK_Status` 映射。

## Benchmarks

[bench](bench/README.zh-CN.md) 目录包含一个小型 pricing benchmark，用于对比纯
JavaScript baseline、生成的 C、生成的 checked C，以及生成的 unchecked WASM。

```sh
pnpm build
pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h
pnpm ikc emit-wasm examples/pricing.ik --out build/pricing.wasm
node bench/pricing_baseline.js
node bench/wasm_pricing_benchmark.mjs
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL build/pricing.c bench/pricing_c_harness.c -I build -o build/pricing_c_bench
./build/pricing_c_bench
```

Benchmark 只是粗略的本地参考。跨语言集成时，应把工作批量放进较大的 native
调用，而不是一条 item 调一次 native 函数。Benchmark README 还包含 unchecked
vs checked C benchmark 命令，并说明 WASM 当前只支持 unchecked。

## 当前 V0 限制

V0 只支持：

- `i32`、`i64`、`u32`、`u64`、`bool`
- `ptr<T>`
- `struct`
- `fn` 和 `export fn`
- `let`、assignment、`return`、`if` / `else`、`while`
- 整数算术、比较、逻辑运算
- 指针索引和 struct 字段访问

V0 不支持字符串、IO、heap allocation、GC、异常、async、class、闭包、模块、
runtime library、LLVM 或 JIT。Phase 12 WASM backend 仍是实验性能力，目前只支持
unchecked mode。

V0 不做 bounds check。默认 arithmetic 是 unchecked；可选
`--overflow checked` C code generation 会检查整数 overflow 和除零，但仍不检查
pointer validity 或 buffer length。Phase 12 WASM backend 会拒绝
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
- [Naming Conventions](docs/NAMING_CONVENTIONS.md)
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
- [命名规范](docs/zh-CN/NAMING_CONVENTIONS.md)
- [路线图](docs/zh-CN/ROADMAP.md)
- [发布检查清单](docs/zh-CN/RELEASE_CHECKLIST.md)
