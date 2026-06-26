# Python ctypes Pricing 示例

[English](README.md)

这个示例只使用 Python 标准库 `ctypes`，调用由 `examples/pricing.ck` 生成的动态库。

## 构建动态库

从仓库根目录先构建 TypeScript CLI：

```sh
pnpm build
```

然后生成并编译 pricing 动态库。

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

## 运行示例

从仓库根目录执行：

```sh
python3 examples/python-ctypes-call/call_pricing.py
```

Windows：

```sh
py examples\python-ctypes-call\call_pricing.py
```

当 `calc_items` 返回 `0`，且 output buffer 符合预期值时，脚本打印 `OK`。

## Checked Mode

Checked mode 使用不同 C ABI。函数返回 `CK_Status`，原始 CalcKernel return value
通过最后一个 pointer 参数写出。

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

运行 checked 示例：

```sh
python3 examples/python-ctypes-call/call_pricing_checked.py
```

Windows：

```sh
py examples\python-ctypes-call\call_pricing_checked.py
```

脚本会对成功 pricing call 打印 `OK`，并对 `price * qty` overflow 的 case 打印
`overflow check OK`。

## ctypes 映射

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

Python binding 精确镜像这个定义：

- `i64` -> `ctypes.c_int64`
- `i32` -> `ctypes.c_int32`
- `ptr<Item>` -> `Item` 的 array 或 pointer
- `ptr<i64>` -> `ctypes.c_int64` 的 array 或 pointer

调用方分配两个 buffer：

- `items = (Item * n)(...)`
- `out = (ctypes.c_int64 * n)(...)`

V0 不分配内存、不释放内存、不做 bounds check，也不检查 integer overflow。调用方
必须传入有效 pointer、有效 length，以及保持在预期整数范围内的值。

## Checked ctypes 映射

Checked 生成的 C header 定义 status values：

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
```

Checked `calc_items` declaration 是：

```c
CK_API CK_Status calc_items(Item* items, int32_t len, int64_t* out, int32_t* ik_return);
```

Python binding 映射为：

```python
CK_Status = ctypes.c_int32
CK_OK = 0
CK_ERR_OVERFLOW = 1
CK_ERR_DIV_BY_ZERO = 2
CK_ERR_NULL_POINTER = 3

lib.calc_items.argtypes = [
    ctypes.POINTER(Item),
    ctypes.c_int32,
    ctypes.POINTER(ctypes.c_int64),
    ctypes.POINTER(ctypes.c_int32),
]
lib.calc_items.restype = ctypes.c_int32
```

调用方仍然分配 `items`、`out` 和 `ik_return`：

```python
out = (ctypes.c_int64 * len(items))(0, 0, 0)
ik_return = ctypes.c_int32()
status = lib.calc_items(items, ctypes.c_int32(len(items)), out, ctypes.byref(ik_return))
```

Checked mode 会检查 integer overflow、division by zero 和生成的 `ik_return` pointer。
它仍然不做 bounds check，不验证用户 data pointer，也不确认 output buffer 是否足够大。
