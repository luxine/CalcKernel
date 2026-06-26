# CalcKernel 语言规格

[English](../LANGUAGE_SPEC.md)

本文描述 CK / CalcKernel V0 语言。

CK / CalcKernel 是一门面向高性能纯计算 kernel 的 DSL。它不是通用编程语言。V0
的目标是将 `.ck` 源码编译成 C、WASM 和 LLVM backend 输出，供宿主语言和 native
toolchain 使用。整数 kernel 仍是主要目标；Phase 16 增加 strict `f64`，用于
数值 kernel。

## 源文件

CalcKernel 源文件使用 `.ck` 扩展名。

## 支持的类型

V0 支持以下类型：

- `i32`
- `i64`
- `u32`
- `u64`
- `f64`
- `bool`
- `ptr<T>`
- `struct`

`ptr<T>` 表示调用方拥有的、指向 `T` 的指针。V0 没有 owned array，也没有动态
分配。

`f64` 是 strict floating point，适合数值 kernel。它是 CK / CalcKernel 唯一
floating point type；不规划 `f32`。金额、税费、POS 总价或 pricing rule 计算不建议
使用 `f64`；这些领域继续推荐 `i64` fixed-point arithmetic，这样 checked integer
mode 可以明确报告 overflow 和 division error。

语言不支持 `f32`、implicit int/float conversion、fast-math、SIMD 或 float checked
overflow。Phase 20 只增加 exact explicit `i32`/`u32` 到 `f64` 的 compiler
builtin cast；其他 cast 方向仍不支持。

## 支持的声明

### Struct

```ck
struct Item {
  price: i64;
  qty: i64;
}
```

Struct 字段有名称和类型。重复的 struct 名称，以及同一个 struct 内的重复字段，
都是类型错误。

### Function

```ck
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

函数有带类型的参数和带类型的返回值。`export fn` 会出现在生成的 C header 中。
非导出的 `fn` 会生成 `static` C 函数，不会声明在 header 中。

## 支持的语句

- `let`
- assignment
- `return`
- `if` / `else`
- `while`
- block statement

示例：

```ck
let i: i32 = 0;
i = i + 1;

if a > b {
  return a;
} else {
  return b;
}

while i < len {
  i = i + 1;
}
```

V0 函数必须在最终语句路径上明确返回。函数结尾没有 return 是类型错误。

## 支持的表达式

- integer literal
- float literal
- boolean literal：`true`、`false`
- variable reference
- function call
- unary operator：`!`、unary `-`
- arithmetic operator：`+`、`-`、`*`、`/`、`%`
- comparison operator：`==`、`!=`、`<`、`<=`、`>`、`>=`
- logical operator：`&&`、`||`
- pointer index access：`items[i]`
- struct field access：`item.price`
- combined access：`items[i].price`
- parentheses

### Float Literal

Float literal 的类型是 `f64`。

支持形式：

- `1.0`
- `0.5`
- `1e3`
- `1.0e-3`
- `2E8`
- `2E+8`

暂不支持：

- `1.`
- `.5`
- `1e`
- `1e+`
- `1.0f64` 这类 suffix
- `1_000.0` 这类 underscore
- `NaN` 或 `Inf` literal 语法

负数不会作为 literal token 的一部分解析。`-1.0` 是 unary minus 加
`FloatLiteral`，不是带符号 literal token。

### Explicit int-to-f64 Cast

CK / CalcKernel 支持两个 exact compiler builtin，用于从 32-bit integer 显式进入
strict `f64` 代码：

```ck
export fn avg_i32(sum: i32, count: i32) -> f64 {
  return i32_to_f64(sum) / i32_to_f64(count);
}

export fn ratio_u32(a: u32, b: u32) -> f64 {
  return u32_to_f64(a) / u32_to_f64(b);
}
```

- `i32_to_f64(i32) -> f64`
- `u32_to_f64(u32) -> f64`

这两个转换是 exact，因为所有 `i32` 和 `u32` 值都可以被 `f64` 精确表示。它们是
compiler builtin，不是 runtime function；名称是 reserved。

以下仍不支持：

- `i64_to_f64`
- `u64_to_f64`
- 任何 `f64_to_*` cast
- overloaded `to_f64(x)`
- `x as f64`
- constructor-like `f64(x)`
- implicit conversion

`let x: f64 = 1` 和 `i32_value + 1.0` 仍然是类型错误。需要转换时应写
`i32_to_f64(i32_value) + 1.0`。如果 explicit cast 的结果参与除以零，结果遵循普通
strict f64 行为，可能产生 infinity 或 NaN；这不是 checked integer error。

### f64 边界语义

CK / CalcKernel 的 f64 语义是 strict 且刻意保持窄范围：

- `NaN` 和 infinity 没有 literal 语法；只能通过 `0.0 / 0.0` 或 `1.0 / 0.0`
  这类 arithmetic 产生。
- `-0.0` 会通过 backend floating point 行为体现，optimizer 不能用代数重写把它
  随意消掉。
- NaN comparison 遵循所选 backend 的普通 IEEE-like 行为；测试用 `isNaN` 分类，
  不比较 NaN payload。
- Infinity 按符号分类。
- 有限值跨 backend e2e 检查使用 absolute tolerance 和 relative tolerance。
- CK / CalcKernel 不保证 C、LLVM、WASM 和 JavaScript host 之间的浮点结果
  bit-identical。

## 运算符优先级

下表从最高优先级列到最低优先级。

| 优先级 | 运算符 / 形式 | 结合性 |
| --- | --- | --- |
| 1 | function call `f(...)`、index `a[i]`、field `a.b` | left |
| 2 | unary `!`、unary `-` | right |
| 3 | `*`、`/`、`%` | left |
| 4 | `+`、binary `-` | left |
| 5 | `<`、`<=`、`>`、`>=` | left |
| 6 | `==`、`!=` | left |
| 7 | `&&` | left |
| 8 | `||` | left |

括号会覆盖默认优先级。

## 类型检查规则

V0 的类型检查刻意保持严格：

- 所有变量引用必须解析到参数或局部变量。
- 函数调用必须解析到已声明函数。
- 函数调用参数数量必须完全匹配。
- 函数调用参数类型必须可赋值给对应参数类型。
- Struct 类型作为命名类型使用前必须已声明。
- Struct 字段访问要求对象是 struct value，且字段存在。
- Index access 要求对象是 pointer value。
- Pointer index expression 必须是 `i32`、`u32` 或 integer literal。
- `if` 和 `while` 条件必须是 `bool`。
- Assignment target 必须是变量、字段或 index expression。
- Assignment value 类型必须可赋值给 target 类型。
- Return value 类型必须可赋值给函数返回类型。
- Arithmetic operator 要求同类型 numeric 操作数。
- 整数 arithmetic 支持 `+`、`-`、`*`、`/` 和 `%`。
- f64 arithmetic 支持 `+`、`-`、`*` 和 `/`。
- `f64 % f64` 会被拒绝。
- Ordered comparison 要求同类型 numeric 操作数。
- Equality comparison 要求兼容操作数类型。
- Logical operator 要求 `bool` 操作数并返回 `bool`。
- Unary `!` 要求 `bool` 并返回 `bool`。
- Unary `-` 要求整数或 `f64` 操作数，并返回相同类型。
- 混合 integer/f64 arithmetic 和 comparison 会被拒绝。
- Integer literal 不会 materialize 成 `f64`。
- Float literal 不会 materialize 成 integer type。

有上下文时，integer literal 会 materialize 成期望整数类型。否则默认是 `i32`。

类型检查器会拒绝以下示例：

```ck
let x: f64 = 1;
let y: i64 = 1.0;
let z: f64 = 1.0 + 2;
let w: bool = 1.0 < 2;
```

## V0 非目标

V0 不支持：

- strings
- IO
- dynamic memory allocation
- heap allocation
- garbage collection
- exceptions
- async
- classes 或 objects
- closures
- modules 或 imports
- runtime library
- 将 checked overflow 作为语言语法特性或默认行为
- bounds checks
- `f32`（不规划）
- `f64 %`
- implicit int/float conversion
- `i64_to_f64` / `u64_to_f64`
- f64-to-int cast
- broad 或 overloaded cast system
- fast-math
- SIMD
- JIT compilation
- `NaN` literal syntax
- `Infinity` literal syntax
- float suffix literal syntax
- cross-backend bit-identical floating point guarantee

## 整数溢出

V0 默认使用 unchecked integer overflow。使用 `--overflow unchecked` 时，生成的
C 会对映射后的 C 类型使用普通 C 整数运算，不插入 overflow 或 division-by-zero
检查。

编译器也支持可选的 checked arithmetic code generation：
`--overflow checked`。Checked mode 会改变生成的 C ABI：函数返回 `CK_Status`，
原始 CalcKernel return value 通过最后一个 `ck_return` 指针写出，并检查整数加、
减、乘、除、取模和 unary minus。它也会报告除零，以及 `INT64_MIN / -1` 这类
signed division/modulo overflow。

Checked arithmetic 是 code generation mode，不是新的 `.ck` 语法。它不添加 bounds
check、pointer validity check、heap allocation、runtime support 或异常。

`f64` arithmetic 不做 overflow check。在 checked C mode 下，f64 operation 使用
普通 strict C `double` 行为：f64 division by zero 不返回 `CK_ERR_DIV_BY_ZERO`，
f64 overflow 不返回 `CK_ERR_OVERFLOW`。完整 ABI 和安全边界见
[Checked Arithmetic](CHECKED_ARITHMETIC.md)。
