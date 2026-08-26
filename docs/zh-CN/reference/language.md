# CK V0.9 语言参考

[English](../../reference/language.md)

本文档是 CalcKernel 0.9 的规范性源码语言 contract。CK 是确定性计算 kernel
语言，源码文件扩展名为 `.ck`。

## 类型与声明

值类型包括 `i32`、`i64`、`u32`、`u64`、`f64`、`bool`、`ptr<T>`、
`slice<T>` 与命名 struct。`void` 只是 return-only type：`-> void` 只可作为
return type，不可作为参数、local、field、pointer/slice element、operand 或 value。
直接的 `slice<slice<T>>` element 与 exported slice return 非法；internal
function 可以返回 slice。

`ptr<T>` 与 `slice<T>` 都引用 caller-owned memory。CK 不分配、释放、保留或
延长该内存的 lifetime。Slice descriptor 包含 typed data pointer 与 `u32` 长度。

Struct 按声明顺序包含具名 typed field；函数具有 typed parameter 与唯一 return
type。`export fn` 产生 backend 对外 symbol，普通 `fn` 仅供内部调用。同一声明
scope 内名称不可重复，命名 struct type 与被调用函数必须能够解析。

```ck
struct Item { value: i32; }

export fn add(a: i64, b: i64) -> i64 {
  return a + b;
}

fn touch(items: slice<Item>) -> void {
  return;
}
```

## Statement 与控制流

支持 typed `let`、assignment、`return`、`if` / `else`、`while`、`break;`、
`continue;`、block，以及 callee 返回 `void` 的 call statement。

`break;` 退出最内层 `while`；`continue;` 跳到该循环的 condition。两者在循环
外使用时为 `CK2009`。同一 block 中 non-fallthrough statement 之后的 statement
为 `CK2010`。非法 void position 使用 `CK2011`，非法 slice shape/operation 使用
`CK2012`。即使 condition 是 literal `true`，checker 也保守地认为循环可能退出。

返回 value 的函数必须在每条最终路径返回值。Void 函数可自然结束或使用
`return;`。Void 函数返回值、非 void 函数空 return、丢弃非 void call 结果，或把
void call 当作 value 都是非法行为。

Assignment target 可以是 local、parameter、field、pointer index 或 slice index。
Slice `.data` 与 `.len` projection 是 read-only；完整 descriptor 可以被赋值。

## Expression

Expression 包括 integer、`f64`、boolean literal，identifier，call，parentheses，
unary `!` / `-`，arithmetic `+ - * / %`，comparison `== != < <= > >=`，
short-circuit `&&` / `||`，field、pointer/slice index、sub-slice 与 slice 构造。

优先级从高到低：call/index/sub-slice/field；unary；`* / %`；`+ -`；ordered
comparison；equality；`&&`；`||`。除 unary 为右结合外均为左结合；括号覆盖默认规则。

Operand、call argument、slice 构造 operand 与 range endpoint 按源码顺序各求值
一次。`&&` / `||` 仅在需要时求值右 operand。

## 严格类型与数值语义

Operator 要求精确兼容的类型，不存在隐式数值转换。Integer literal 在有 context
时物化为期望 integer type，否则默认 `i32`；float literal 的类型是 `f64`。
Integer 支持 `+ - * / %`，`f64` 支持 `+ - * /`，不支持 `%`。

唯一转换是保留的 compiler builtin：`i32_to_f64(i32) -> f64` 与
`u32_to_f64(u32) -> f64`，两者均精确。没有 integer-width cast、f64-to-int、
隐式 cast、`as` 或 constructor-style cast。

带小数点的 float literal 必须在点两侧都有 digit，并支持 exponent。`NaN` 与
infinity 没有 literal syntax，但可由运算产生。Backend 保持普通严格 double
precision 行为，包括 signed zero 与 NaN unordered comparison；不承诺跨 backend
bit-identical 浮点结果。

默认生成 unchecked integer code。`--overflow checked` 是 C backend 的错误报告
mode，不是源码 syntax，也不检查浮点运算。

## Raw pointer 与 slice

Pointer index 接受 `i32`、`u32` 或 context-compatible integer literal，不做 CK
validity/bounds check。

`slice(data, len)` 从 `ptr<T>` 与 `u32` length 构造 `slice<T>`。内存有效性、
alignment、allocation extent、lifetime 与声明长度真实性仍由 caller 负责；复制
descriptor 会 alias 同一内存。

`items[index]` 要求 `u32` index。`items[start..end]` 创建半开 sub-slice；两个
endpoint 都是 `u32`，有效执行必须满足 `start <= end <= items.len`。`.data`
返回 `ptr<T>`，`.len` 返回 `u32`。

只有选择 `--bounds checked` 时，生成的 C 才进行 slice bounds checking。
Raw pointer indexing、slice construction 以及通过 `.data` indexing 永远不会由 CK 验证。
Unchecked C、WASM、LLVM 不生成 guard；WASM 与 LLVM 拒绝 checked bounds。

## Diagnostic 与非目标

Lexing、parsing、type checking 使用[诊断参考](diagnostics.md)中的稳定 code。
V0.9 不提供 string、I/O、module/import、dynamic allocation、ownership runtime、
exception、async、class、closure、`f32`、SIMD、GPU target、JIT 或源码级 checked
operator。
