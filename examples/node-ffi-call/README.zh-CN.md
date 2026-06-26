# Node.js FFI Pricing 示例

[English](README.md)

这个示例通过 Node.js 调用由 `examples/pricing.ck` 生成的动态库。它刻意和根项目隔离，
这样主编译器 package 不需要依赖 native FFI module。

示例在本目录内使用轻量 Node.js C FFI package `koffi`。

## 构建动态库

从仓库根目录构建 CalcKernel CLI：

```sh
pnpm build
```

然后编译 pricing kernel。

macOS：

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing
```

生成：

```text
build/libpricing.dylib
```

Linux：

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing
```

生成：

```text
build/libpricing.so
```

Windows：

```sh
pnpm ckc build examples/pricing.ck --out build/pricing.dll
```

生成：

```text
build/pricing.dll
```

## 安装并运行

在本目录内安装示例依赖：

```sh
cd examples/node-ffi-call
pnpm install
pnpm start
```

也可以使用 npm：

```sh
cd examples/node-ffi-call
npm install
npm start
```

当 `calc_items` 返回 `0`，且 output buffer 匹配预期值时，脚本打印 `OK`。

## Checked Mode

Checked mode 使用不同 C ABI。函数返回 `CK_Status`，原始 CalcKernel return value
通过最后一个 output pointer 写出。

从仓库根目录构建 checked dynamic library。

macOS：

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
```

生成：

```text
build/libpricing_checked.dylib
```

Linux：

```sh
pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked
```

生成：

```text
build/libpricing_checked.so
```

Windows 上，显式传入期望 `.dll` 文件名：

```sh
pnpm ckc build examples/pricing.ck --out build/pricing_checked.dll --overflow checked
```

生成：

```text
build/pricing_checked.dll
```

从本目录运行 checked 示例：

```sh
pnpm start:checked
```

或直接运行：

```sh
node checked.mjs
```

成功调用和 `price * qty` 返回 `CK_ERR_OVERFLOW` 的 overflow case 之后，脚本打印 `OK`。

## FFI 映射

生成的 C header 定义：

```c
typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;

CK_API int32_t calc_items(Item* items, int32_t len, int64_t* out);
```

Node.js binding 使用 Koffi 镜像：

- `i64` / `int64_t` -> Koffi `int64_t`；本示例用 `BigInt` 传值。
- `i32` / `int32_t` -> Koffi `int32_t`；`len` 这类小 32-bit 值使用 JavaScript
  `number`。
- `struct Item` -> `koffi.struct("Item", { ... })`。
- `ptr<Item>` 作为 `Item` 形状对象的 JavaScript array 传入。
- `ptr<i64>` 作为调用方拥有的 `BigInt64Array` output buffer 传入。

## Checked FFI 映射

Checked 生成的 header 定义 status values：

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
```

Checked `calc_items` declaration 是：

```c
CK_API CK_Status calc_items(Item* items, int32_t len, int64_t* out, int32_t* ck_return);
```

Koffi signature 镜像该 ABI：

```js
const calcItems = lib.func("int32_t calc_items(Item *items, int32_t len, _Out_ int64_t *out, _Out_ int32_t *ck_return)");
```

Checked 示例使用：

- `number` 表示 `CK_Status`、`CK_OK` 和其他 32-bit status constants。
- `Int32Array(1)` 表示最后的 `ck_return` pointer。
- 每个 `Item` 的 `int64_t` 字段都使用 `BigInt`。
- `BigInt64Array` 表示 `int64_t* out` buffer。

这样可以避免把 `i64` value 默默转换成不安全的 JavaScript `number`。真实集成中，
对 `i64` / `u64` 值继续使用 `BigInt` 或 Koffi 支持的 64-bit 表示。

## V0 安全说明

V0 不分配内存、不释放内存，也不做 bounds check。Checked mode 会检查 integer
overflow、division by zero 和生成的 `ck_return` pointer，但仍不验证用户 data
pointer 或 buffer length。调用方必须传入有效 buffer，以及匹配已分配 array 的
length。如果 length 错误，native code 的风险和等价 C pointer indexing 一样。
