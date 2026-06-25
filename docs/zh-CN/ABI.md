# IntKernel C ABI

[English](../ABI.md)

IntKernel V0 目标是 plain C ABI。生成的 `.h` 和 `.c` 文件应由 C 编译器编译，
并通过 C、C++、Python、Node.js、Rust、Go、C# 等宿主语言的常规 FFI 机制消费。

V0 不提供 runtime。它不分配内存、不释放内存，也不拥有任何跨 ABI 传入指针的
生命周期。调用方拥有每个输入和输出 buffer。

## 跨 Backend f64 摘要

Phase 16 f64 strict mode 使用以下 ABI 映射：

| Backend | Scalar `f64` | `ptr<f64>` | Struct field `f64` |
| --- | --- | --- | --- |
| C | `double` | `double*` | C `double` field |
| LLVM | `double` | opaque `ptr` + `getelementptr double` | LLVM `double` field |
| WASM | `f64` value type | `i32` byte offset，8-byte element step | deterministic size 8 / align 8 field |

JavaScript WASM interop 对 f64 参数和返回值使用 `Number`，不使用 `BigInt`。
WASM host memory 中的 `ptr<f64>` 使用 little-endian `DataView.setFloat64` 和
`DataView.getFloat64` 读写。

WASM deterministic layout 中 f64 的 size 是 8、alignment 是 8。C ABI 使用目标
C 编译器的 `double` layout；当前 release targets 覆盖的测试预期 size 8、
alignment 8。IK / IntKernel 不承诺所有 C、LLVM、WASM 和 JavaScript target 的浮点
结果 bit-identical。

f64 语义锁定：

- C 使用普通 `double` operation。
- LLVM 生成不带 fast-math flag 的 `double` operation。
- WASM 生成 `f64` operation，scalar f64 通过 JavaScript `Number` 暴露给 host。
- NaN、infinity 和 `-0.0` 遵循 backend 的普通 IEEE-like 行为。
- NaN payload 和跨 backend bit identity 不属于 ABI contract。
- 有限值跨 backend 测试必须使用 tolerance；NaN、infinity、signed zero 和 bool
  comparison result 必须显式分类。

## 类型映射

| IntKernel type | C ABI type |
| --- | --- |
| `i32` | `int32_t` |
| `i64` | `int64_t` |
| `u32` | `uint32_t` |
| `u64` | `uint64_t` |
| `f64` | `double` |
| `bool` | `bool` |
| `ptr<T>` | `T*` |
| `struct` | `typedef struct` |

示例：

```ik
struct Item {
  price: i64;
  qty: i64;
}
```

生成：

```c
typedef struct Item {
  int64_t price;
  int64_t qty;
} Item;
```

## Header ABI

生成的 header 使用 `#pragma once`，并包含标准 integer 和 bool header。Checked
mode header 还会包含 `stddef.h`，用于 `NULL`：

```c
#pragma once

#include <stdint.h>
#include <stdbool.h>
/* checked mode also emits: #include <stddef.h> */
```

导出函数使用 `IK_API` 宏：

```c
#if defined(_WIN32) || defined(__CYGWIN__)
  #ifdef IK_BUILD_DLL
    #define IK_API __declspec(dllexport)
  #else
    #define IK_API __declspec(dllimport)
  #endif
#else
  #define IK_API __attribute__((visibility("default")))
#endif
```

将生成的 C 编译成动态库时，需要定义 `IK_BUILD_DLL`。这会在 Windows 上把导出
函数标记为库定义，同时在各平台保持一致的 build contract。

生成的 header 也可以安全地被 C++ translation unit include：

```c
#ifdef __cplusplus
extern "C" {
#endif

/* typedef structs and IK_API function declarations */

#ifdef __cplusplus
}
#endif
```

`extern "C"` block 防止导出函数发生 C++ name mangling。

例如：

```ik
export fn calc(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32
```

会生成：

```c
IK_API int32_t calc(Item* items, int32_t len, int64_t* out);
```

`f64` scalar 映射为 C `double`，`ptr<f64>` 映射为 `double*`：

```ik
export fn scale(value: f64, out: ptr<f64>) -> f64
```

生成的 header signature 形态为：

```c
IK_API double scale(double value, double* out);
```

非导出的 `fn` 不会出现在 header 中。它们在生成的 `.c` 文件中生成为 `static`
函数，不属于 public ABI。

## Dynamic Libraries

CLI `build` 命令生成 `.c` 和 `.h` 文件，然后用严格 flags 和 `-DIK_BUILD_DLL`
调用 clang。

平台输出名称：

| Platform | Extension | Example |
| --- | --- | --- |
| macOS | `.dylib` | `build/libpricing.dylib` |
| Linux | `.so` | `build/libpricing.so` |
| Windows | `.dll` | `build/pricing.dll` |

macOS：

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL \
  -shared -fPIC pricing.c \
  -o libpricing.dylib
```

Linux：

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL \
  -shared -fPIC pricing.c \
  -o libpricing.so
```

Windows：

```sh
clang -std=c11 -O3 -Wall -Wextra -Werror -DIK_BUILD_DLL \
  -shared pricing.c \
  -o pricing.dll
```

## Struct Layout

IntKernel 会严格保留 `.ik` 源码中的 struct 字段顺序。V0 使用目标 C 编译器的
自然 struct alignment rules。它不生成 packed struct、`#pragma pack` 或自定义
alignment attribute。

FFI binding 必须用和生成 header 相同的字段顺序、字段类型和 C layout 定义宿主侧
struct。不要假设 packed layout。
`f64` 字段使用目标 C 编译器的 `double` size 和 alignment；当前 release targets
覆盖的测试预期 size 8、alignment 8。C ABI 有意遵循 C 编译器 layout，而不是承诺
跨平台固定 binary layout。

对 `examples/pricing.ik`，生成的 `Item` layout 是：

| Field | C type | Offset |
| --- | --- | --- |
| `price` | `int64_t` | 0 |
| `qty` | `int64_t` | 8 |
| `discount` | `int64_t` | 16 |
| `tax_rate_ppm` | `int64_t` | 24 |

在 V0 C ABI 测试中，该示例的 `sizeof(Item)` 预期为 32。

## Buffer Ownership

跨 ABI 的所有内存都由调用方负责：

- 分配 input buffers
- 分配 output buffers
- 在整个调用期间保持 buffer 存活
- 传入有效 pointer
- 传入有效 `len`
- 确保 output buffer 足够容纳函数写入

IntKernel 生成的函数只读写调用方传入的内存。它不分配替代 buffer，也不会在调用
返回后保存 pointer。

这就是示例使用 `out: ptr<i64>` 这类 output pointer，而不是返回已分配数组的原因。

## 安全限制

V0 刻意接近 C：

- 默认 arithmetic unchecked
- pointer indexing 无 bounds check
- invalid pointer 是 undefined behavior
- invalid length 是 undefined behavior
- output buffer 太小是 undefined behavior
- unchecked mode 下，division by zero 遵循生成 C 的行为
- checked mode 下，arithmetic overflow 和 division by zero 返回 `IK_Status`
  错误，但 memory safety 仍由调用方负责
- `f64` 映射为 C `double`，C backend 支持 scalar f64 arithmetic、comparison、
  `ptr<f64>` 和 struct field；checked mode 不增加 floating overflow 或 floating
  division-by-zero error

调用方和 DSL 作者必须选择足够宽的 integer type、验证 length，并传入正确 buffer。

## Checked Arithmetic ABI

Unchecked arithmetic 仍是默认 ABI。使用 `--overflow unchecked` 时，导出函数保留
原始 C signature，非导出函数保留原始 `static` signature，表达式直接生成 C。

Phase 10 引入可选 checked arithmetic mode：

```sh
ikc emit-c input.ik --out build/input.c --header build/input.h --overflow checked
ikc build input.ik --out build/libinput --overflow checked
```

Checked mode 改变生成的 C ABI。导出函数返回 `IK_Status`，source-level return
value 通过最后一个生成的 output pointer `ik_return` 写出：

```c
typedef int32_t IK_Status;

#define IK_OK ((IK_Status)0)
#define IK_ERR_OVERFLOW ((IK_Status)1)
#define IK_ERR_DIV_BY_ZERO ((IK_Status)2)
#define IK_ERR_NULL_POINTER ((IK_Status)3)
```

对 source function：

```ik
export fn add_i64(a: i64, b: i64) -> i64
```

unchecked mode 生成：

```c
IK_API int64_t add_i64(int64_t a, int64_t b);
```

checked mode 生成：

```c
IK_API IK_Status add_i64(int64_t a, int64_t b, int64_t* ik_return);
```

签名规则：

- C return type 变为 `IK_Status`
- 原始参数按顺序保留
- 追加最后一个 `T* ik_return` 参数，`T` 是原 IntKernel return type 映射后的 C 类型
- 如果 `ik_return == NULL`，生成的 checked code 返回 `IK_ERR_NULL_POINTER`
- 成功时，生成的 checked code 将原始 return value 写入 `*ik_return`，并返回 `IK_OK`

示例：

```ik
export fn calc_items(items: ptr<Item>, len: i32, out: ptr<i64>) -> i32
```

checked mode 生成：

```c
IK_API IK_Status calc_items(
  Item* items,
  int32_t len,
  int64_t* out,
  int32_t* ik_return
);
```

Checked mode 中，非导出 IntKernel 函数也使用 checked lowering，但仍然只在生成的
`.c` 文件内私有：

```c
static IK_Status helper(int64_t a, int64_t* ik_return);
```

调用方不会直接调用非导出 helper。

Checked mode 报告 integer arithmetic overflow、integer division by zero、
`INT64_MIN / -1` 这类 signed integer division 或 modulo overflow，以及
`-INT64_MIN` 这类 integer unary minus overflow。`f64` arithmetic 在 checked mode
下使用普通 C `double` 行为：f64 division by zero 和 f64 overflow 不返回
`IK_ERR_DIV_BY_ZERO` 或 `IK_ERR_OVERFLOW`。它不添加 pointer bounds check，也不自动
检查用户传入的 `ptr<T>` 参数。

因为 checked mode 改变 signature，unchecked 和 checked dynamic library 应视为不同
ABI artifact。

完整 Phase 10 设计见 [Checked Arithmetic Design](CHECKED_ARITHMETIC.md)。

## 跨语言调用说明

### C

Include 生成的 header，并链接生成的 object file 或 dynamic library。使用 header
中显示的精确 C 类型。

### C++

生成的 header 包含 `extern "C"` guard，因此 C++ 代码可以直接 include，导出函数
不会被 name mangling。

### Python ctypes

用 `ctypes.Structure` 镜像生成的 struct。映射：

- `i32` / `int32_t` -> `ctypes.c_int32`
- `i64` / `int64_t` -> `ctypes.c_int64`
- `u32` / `uint32_t` -> `ctypes.c_uint32`
- `u64` / `uint64_t` -> `ctypes.c_uint64`
- `ptr<T>` -> `ctypes.POINTER(T)` 或调用方拥有的 ctypes array

Checked function 设置 `restype = ctypes.c_int32` 表示 `IK_Status`，在 `argtypes`
末尾追加原始 return value 的 pointer，用 `ctypes.byref(...)` 传入，并在读取值前检查
返回 status。

见 `examples/python-ctypes-call`。

### Node.js

JavaScript `number` 无法精确表示所有 `i64` 或 `u64` 值。64-bit integer value 优先
使用 `BigInt` 或 typed/native buffer。Koffi 示例用 `BigInt` 表示 `int64_t` 字段，
用 `BigInt64Array` 表示 `ptr<i64>` 输出 buffer。

Checked function 将 C return 绑定为 `int32` status，并额外传入原始 IntKernel
return value 的 pointer 参数。读取 output buffer 或生成的 return pointer 前检查
`IK_OK`。

见 `examples/node-ffi-call`。

### Rust

镜像 struct 使用 `#[repr(C)]`，并使用 `std::os::raw` 中的 C-compatible integer
type，或 `i32`、`i64`、`u32`、`u64` 这类 fixed-width Rust primitive。根据生成函数
读写方式，将 pointer 表示为 `*const T` 或 `*mut T`。

### Go cgo

用 cgo include 生成的 header 并链接生成的 library。尽量通过 C type 镜像 struct
layout，或直接使用 cgo 生成的 C struct type，以避免 layout drift。

### C# P/Invoke

使用 `[DllImport]` 绑定导出函数。Struct 使用
`[StructLayout(LayoutKind.Sequential)]` 镜像，字段使用和生成 C ABI 类型对应的
fixed-width integer type，例如 `int`、`long`、`uint`、`ulong`。

## 推荐调用模式

将工作批量放进大调用：

```c
calc_items(items, len, out);
```

除非你已经测量过调用开销可接受，否则不要每个 item 跨 host/native 边界调用一次。
对 Python、Node.js、C# 等 FFI 用户来说，每次调用的 overhead 可能压过 kernel 的
计算时间。优先传入 array 和 output buffer，让生成的 C 代码在一次 native call 内
执行 loop。
