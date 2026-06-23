# IntKernel 语言规格

[English](../LANGUAGE_SPEC.md)

本文描述 IntKernel V0 语言。

IntKernel 是一门用于纯整数计算的小型 DSL。它不是通用编程语言。V0 的目标是
将 `.ik` 源码编译成可读的 C source 和 header，再由宿主平台编译成本地库，供
宿主语言调用。

## 源文件

IntKernel 源文件使用 `.ik` 扩展名。

## 支持的类型

V0 只支持以下类型：

- `i32`
- `i64`
- `u32`
- `u64`
- `bool`
- `ptr<T>`
- `struct`

`ptr<T>` 表示调用方拥有的、指向 `T` 的指针。V0 没有 owned array，也没有动态
分配。

## 支持的声明

### Struct

```ik
struct Item {
  price: i64;
  qty: i64;
}
```

Struct 字段有名称和类型。重复的 struct 名称，以及同一个 struct 内的重复字段，
都是类型错误。

### Function

```ik
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

```ik
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
- Arithmetic operator 要求同类型整数操作数。
- Ordered comparison 要求同类型整数操作数。
- Equality comparison 要求兼容操作数类型。
- Logical operator 要求 `bool` 操作数并返回 `bool`。
- Unary `!` 要求 `bool` 并返回 `bool`。
- Unary `-` 要求整数操作数并返回相同整数类型。

有上下文时，integer literal 会 materialize 成期望整数类型。否则默认是 `i32`。

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
- LLVM backend
- WASM backend
- JIT compilation

## 整数溢出

V0 默认使用 unchecked integer overflow。使用 `--overflow unchecked` 时，生成的
C 会对映射后的 C 类型使用普通 C 整数运算，不插入 overflow 或 division-by-zero
检查。

编译器也支持可选的 checked arithmetic code generation：
`--overflow checked`。Checked mode 会改变生成的 C ABI：函数返回 `IK_Status`，
原始 IntKernel return value 通过最后一个 `ik_return` 指针写出，并检查整数加、
减、乘、除、取模和 unary minus。它也会报告除零，以及 `INT64_MIN / -1` 这类
signed division/modulo overflow。

Checked arithmetic 是 code generation mode，不是新的 `.ik` 语法。它不添加
bounds check、pointer validity check、heap allocation、runtime support 或异常。
完整 ABI 和安全边界见 [Checked Arithmetic](CHECKED_ARITHMETIC.md)。
