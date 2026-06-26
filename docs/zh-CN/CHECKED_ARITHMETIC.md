# Checked Arithmetic 设计

[English](../CHECKED_ARITHMETIC.md)

## 目标

CalcKernel V0 默认使用 unchecked arithmetic。Phase 10 增加一个可选 checked
arithmetic code generation mode，用于需要更安全整数行为的 kernel。

Checked mode 面向金额、税费、优惠、定价规则等整数密集领域。这些场景需要显式
报告 overflow 或 division by zero，而不是默默落到生成 C 的行为。

Checked arithmetic 目前适用于 C backend（`emit-c` 和 `build`）。Phase 12 WASM
backend 只支持 unchecked：`emit-wat --overflow checked` 和
`emit-wasm --overflow checked` 会用清晰 diagnostic 失败。Phase 13 LLVM backend
也只支持 unchecked：`emit-llvm --overflow checked` 和
`build-llvm --overflow checked` 会用清晰 diagnostic 失败。Checked WASM 和
checked LLVM lowering 属于未来工作。

## CLI 设计

Unchecked mode 仍是默认：

```sh
ckc emit-c input.ck --out build/input.c --header build/input.h
ckc build input.ck --out build/libinput
```

显式形式：

```sh
ckc emit-c input.ck --out build/input.c --header build/input.h --overflow unchecked
ckc emit-c input.ck --out build/input.c --header build/input.h --overflow checked

ckc build input.ck --out build/libinput --overflow unchecked
ckc build input.ck --out build/libinput --overflow checked
```

默认值：

```text
--overflow unchecked
```

## Unchecked Mode

Unchecked mode 保持当前 V0 行为：

- C ABI 不变。
- 表达式直接生成 C expression。
- 不检查 integer overflow。
- 不检查 division by zero。
- overhead 最低。
- 调用方和 DSL 作者负责保证输入合法。

Unchecked mode 保持原始 C ABI。默认 backend 变化时，生成的 C source snapshot 可能
变化，但 header ABI snapshot 应保持稳定，除非有意做 ABI 变更。

## Unchecked vs Checked

| 主题 | `--overflow unchecked` | `--overflow checked` |
| --- | --- | --- |
| 默认 | 是 | 否 |
| C ABI | 原始 return type | `CK_Status` return 加最后 return pointer |
| Integer overflow | 不检查 | 返回 `CK_ERR_OVERFLOW` |
| Division by zero | 不检查 | 返回 `CK_ERR_DIV_BY_ZERO` |
| `f64` overflow 和 division by zero | C `double` 行为 | C `double` 行为，除非发生其他 checked error，否则仍返回 `CK_OK` |
| User pointers | 不检查 | 不检查，除了生成的 `ck_return` |
| Bounds checks | 无 | 无 |
| Runtime dependency | 无 | 无 |
| 性能 | 最快 | 额外检查和分支 |

## Checked Mode

Checked mode 改变导出函数 ABI：

- exported function 返回 `CK_Status`
- 原始 return value 通过最后一个 output pointer 写出
- 生成的 C 在 overflow、division by zero 或 checked return pointer 为 null 时提前返回
- 生成的 C 自包含
- 不需要 runtime library
- 不使用 exceptions
- 不使用 `setjmp` / `longjmp`

Checked mode 是 code generation mode，不是新语言功能。

Checked mode 下，所有 CalcKernel 函数都使用 checked ABI。导出函数在生成 header 中
以 `CK_API` 出现；非导出函数在生成 `.c` 文件中生成为 `static CK_Status` helper。

从 Phase 11 开始，checked C generation 基于 MIR pipeline：

```text
Typed Program -> MIR lowering -> MIR validator -> MIR C backend
```

MIR 表示普通 typed arithmetic、call、place 和 control flow。Checked MIR C backend
插入 overflow guard、division check、`CK_Status` propagation 和 return-pointer
处理，同时保持 checked ABI。

## Status Values

Checked header 定义：

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
```

- `CK_OK`：计算成功。
- `CK_ERR_OVERFLOW`：checked arithmetic 检测到 overflow。
- `CK_ERR_DIV_BY_ZERO`：division 或 modulo divisor 为零。
- `CK_ERR_NULL_POINTER`：生成的 checked return pointer `ck_return` 为 `NULL`。

## Checked ABI 示例

CalcKernel 源码：

```ck
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

Checked header：

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)

CK_API CK_Status add_i64(int64_t a, int64_t b, int64_t* ck_return);
```

Checked implementation：

```c
CK_Status add_i64(int64_t a, int64_t b, int64_t* ck_return) {
  if (ck_return == NULL) {
    return CK_ERR_NULL_POINTER;
  }

  int64_t ik_tmp0;
  if (__builtin_add_overflow(a, b, &ik_tmp0)) {
    return CK_ERR_OVERFLOW;
  }

  *ck_return = ik_tmp0;
  return CK_OK;
}
```

## Checked Operations

### `+`

- 执行 checked addition。
- Signed 和 unsigned integer type 都检查。
- Overflow 返回 `CK_ERR_OVERFLOW`。

### `-`

- 执行 checked subtraction。
- Signed 和 unsigned integer type 都检查。
- Overflow 返回 `CK_ERR_OVERFLOW`。

### `*`

- 执行 checked multiplication。
- Signed 和 unsigned integer type 都检查。
- Overflow 返回 `CK_ERR_OVERFLOW`。

### `/`

- 对 integer operands，divisor 为零时，返回 `CK_ERR_DIV_BY_ZERO`。
- 对 signed integer，`INT32_MIN / -1` 和 `INT64_MIN / -1` 返回
  `CK_ERR_OVERFLOW`。
- Unsigned division 只需要 zero-divisor check。
- 对 `f64`，执行普通 C `double` division。`1.0 / 0.0` 不返回
  `CK_ERR_DIV_BY_ZERO`。
- 其他情况执行普通 division。

### `%`

- Divisor 为零时，返回 `CK_ERR_DIV_BY_ZERO`。
- 对 signed integer，`INT32_MIN % -1` 和 `INT64_MIN % -1` 返回
  `CK_ERR_OVERFLOW`。
- Unsigned modulo 只需要 zero-divisor check。
- 其他情况执行普通 modulo。

### Unary `-`

- 对 signed integer，`-INT32_MIN` 和 `-INT64_MIN` 返回 `CK_ERR_OVERFLOW`。
- 对 unsigned integer，unary minus 降低为从零开始的 checked subtraction，所以任意
  非零值都会 overflow。
- 对 `f64`，执行普通 C `double` negation。
- 其他情况执行普通 negation。

### `f64`

Checked C mode 仍然是 integer checked arithmetic mode。选择 `--overflow checked`
时，包含 `f64` 的函数也使用 checked ABI，因此返回 `CK_Status`，并通过 `ck_return`
写出 source-level return value。f64 operation 本身使用普通 strict C `double` 行为：

- `f64 +`、`-`、`*`、`/` 不调用 integer overflow builtin。
- f64 division by zero 不返回 `CK_ERR_DIV_BY_ZERO`。
- f64 overflow 不返回 `CK_ERR_OVERFLOW`。
- `i32_to_f64` 和 `u32_to_f64` cast 不返回 `CK_ERR_OVERFLOW`；它们是进入普通
  strict f64 value 的 exact explicit conversion。
- f64 NaN、infinity 和 `-0.0` 是普通 floating point 结果，不是 checked
  arithmetic status value。
- `f64 %` 不是语言操作，会在 C emission 前被拒绝。

这个边界是有意设计：checked mode 保护 integer arithmetic。它不会把 floating
point 变成 trapping 或 exact decimal arithmetic mode。金额、税费、POS 总价和
pricing rule 需要精确 checked failure reporting 时，仍应使用 `i64` fixed-point
arithmetic。

## Logical Operators

`&&` 和 `||` 必须保持源码语言的 short-circuit 语义。

示例：

```ck
a != 0 && b / a > 0
```

如果 `a == 0`，右侧不能求值，因此 `b / a` 不能触发 division-by-zero error。

Phase 11 MIR lowering 将 `&&` 和 `||` 表示成 control flow。Checked MIR C backend
按这些 MIR block 生成代码，因此 right-hand side 不会在决定是否需要它的 branch
之前被生成或求值。

## Function Calls

Checked mode 下，调用另一个 CalcKernel 函数使用 checked ABI：

- 传入原始参数
- 传入临时变量地址接收 callee return value
- 检查返回的 `CK_Status`
- 如果 status 不是 `CK_OK`，当前函数直接返回该 status
- 否则使用临时值作为 call expression result

概念上：

```c
int64_t ik_tmp0;
CK_Status ik_status0 = add_i64(a, b, &ik_tmp0);
if (ik_status0 != CK_OK) {
  return ik_status0;
}
```

Function argument 本身也是 checked expression。例如：

```ck
return add(a + 1, b * 2);
```

会先检查 `a + 1` 和 `b * 2`，再把临时值传给 `add`。如果 callee 返回任何非
`CK_OK` status，caller 立即返回相同 status。

在 MIR 中，call expression 是带 result temporary 的显式 `Call` instruction。
Checked C emission 会把该 instruction 降低成 checked ABI call，传入 `&temporary`
作为最后 return pointer，检查返回的 `CK_Status`，并传播任何非 `CK_OK` status。

## Pointer、Index 和 Field Access

Checked mode 支持 V0 pointer、index 和 struct field access：

```ck
items[i].price
items[i].qty
out[i] = value;
```

生成代码会通过 checked expression lowering path evaluate index expression。如果
index expression 包含 arithmetic，该 arithmetic 会在 pointer access 生成前被检查：

```ck
items[i + 1].price
```

这个例子中，`i + 1` 可能返回 `CK_ERR_OVERFLOW`。

Phase 10 仍不添加 bounds checking。

原因：

- V0 有 `ptr<T>`，但没有携带长度的 pointer type。
- 编译器无法可靠判断 `items[i]` 是否越界。

Checked mode 不检查：

- `items[i]` bounds
- `out[i]` bounds
- 用户传入 pointer 是否有效
- 用户传入 buffer 对 `len` 是否足够长

调用方负责：

- 在 kernel 通过 `ptr<T>` 读写时传入有效 pointer
- 确保 kernel 使用的每个 index 都在范围内
- 确保 output buffer 足够大
- 确保 pointer lifetime 覆盖整个 native call

未来 bounds checking 需要语言级设计，例如 `slice<T>` 或显式 pointer-plus-length
metadata。

## Null Pointers

Phase 10 只检查生成的 checked ABI return pointer `ck_return` 是否为 `NULL`。

它不会自动检查每个用户 `ptr<T>` 参数。

原因：

- 当 `len == 0` 时，某些 API 可能允许 data pointer 为 `NULL`。
- V0 没有 `in`、`out` 或 `nonnull` annotation。
- 自动检查所有 pointer 会改变用户可见语义。

用户 pointer validity 仍由调用方负责。

## 编译器要求

Checked mode 实现目前依赖 Clang/GCC 风格 overflow builtins：

- `__builtin_add_overflow`
- `__builtin_sub_overflow`
- `__builtin_mul_overflow`

当前项目 build path 使用 clang。如果 CalcKernel 未来要支持没有 clang-compatible
builtins 的原生 MSVC 编译，backend 应增加 portable fallback 或 MSVC-specific
lowering，用于 checked add、subtract 和 multiply。

Division、modulo 和 unary minus check 可以直接使用 C comparison，对 type-specific
min value 和 divisor 进行检查。

## 性能影响

Checked mode 预计比 unchecked mode 慢。Overhead 来自：

- overflow builtin call 或等价 compiler-lowered check
- division-by-zero branch
- signed division/modulo overflow branch
- CalcKernel function call 后的 `CK_Status` check
- 额外 temporary 和最终 `ck_return` 写入

当 correctness 和显式 arithmetic failure reporting 比最大吞吐更重要时使用 checked
mode，例如金额、税费、优惠和规则引擎。当调用方或前置验证已经证明不会 overflow
或 division by zero 时，热路径可使用 unchecked mode。

## 限制

Checked mode 不提供完整 memory safety：

- no bounds check
- no runtime
- no heap allocation
- no exceptions
- no checked pointer lifetime
- no checked buffer length
- no checked user-provided output buffers
- checked mode changes the C ABI
- checked mode may be slower than unchecked mode

Checked arithmetic 改善整数错误报告，但不会让 pointer-based kernel 变成 memory safe。
