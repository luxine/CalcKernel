# CalcKernel 0.10 语言参考

[English](../../reference/language.md)

本文档是 CalcKernel 0.10 源语言的规范性契约。CK 是确定性的计算内核语言，源文件扩展名为 `.ck`。

## 类型与声明

值类型包括 `i32`、`i64`、`u32`、`u64`、`f64`、`bool`、`ptr<T>`、
`slice<T>` 和具名 struct。`void` 是 return-only type；`-> void` 合法，但不能成为参数、local、field、
pointer/slice element、operand 或 value。直接嵌套的 `slice<slice<T>>` 与 exported
slice return 非法；internal function 可以返回 slice。

Pointer 与 slice 指向 caller-owned memory。CK 不分配、释放、持有或延长其生命周期。
Slice descriptor 由 typed data pointer 与 `u32` length 组成；struct field 保持声明顺序。
`export fn` 产生 Native、C 或 WebAssembly library export，普通 `fn` 为 internal。

## 入口与 Native 输出

`main` 是保留名称，只允许以下无参数且非 exported 的形式：

```ck
fn main() -> void
fn main() -> i32
```

`ckc run` 与 `ckc build --kind executable` 必须具有 `main`。Void entry 返回进程状态
0；i32 entry 提供平台进程状态。可移植程序使用 0–239，因为 240–245 保留给 runtime failure。

Native entry/runtime-effect model 预声明并保留下列 compiler builtin：

- `print_i32(i32) -> void`、`print_i64(i64) -> void`
- `print_u32(u32) -> void`、`print_u64(u64) -> void`
- `print_f64(f64) -> void`、`print_bool(bool) -> void`
- `print_newline() -> void`

Argument 按源码顺序各求值一次。Native executable 与 `run` root 可以到达这些调用；
Native library/object export、C artifact root 或 WebAssembly export 可达的 print 会被拒绝。
不可达 print 可以被删除。0.10 不提供通用 string 或 byte I/O。

Value print 不追加 newline；`print_newline` 在所有平台精确输出一个 LF。Integer 使用 base 10，
无 locale、grouping、leading zero 或 positive sign。Boolean 为 `true`/`false`。Finite f64 在
round-to-nearest、ties-to-even 下使用 no-allocation shortest-round-trip decimal；negative zero
为 `-0.0`，特殊值为 `nan`、`inf`、`-inf`，不表达 NaN payload/sign。每个 print 都是有序
observable effect。Output failure 以 `CKR0005` 终止进程，不返回 `CK_Status`。

## 语句与控制流

语句包括 typed `let`、assignment、`return`、`if`/`else`、`while`、`break;`、
`continue;`、block 与返回 void 的 call statement。`break` 退出最内层 loop，
`continue` 跳到其 condition。Loop 外使用为 `CK2009`；同一 block 中 non-fallthrough
statement 后的代码为 `CK2010`。非法 void 使用为 `CK2011`，非法 slice shape/operation 为
`CK2012`。分析保守地认为 loop 可以退出。

非 void function 必须在每条终止路径返回值；void function 可以自然结束或 `return;`。
赋值目标可以是 local、parameter、field、pointer index 或 slice index。`.data` 与 `.len`
projection 为 read-only，但整个 slice descriptor 可以赋值。

## 表达式与求值

表达式包含 literal、identifier、call、parentheses、unary、arithmetic、comparison、
short-circuit boolean、field、pointer/slice index、sub-slice 与 slice construction。
优先级从高到低为 postfix、unary、`* / %`、`+ -`、顺序比较、相等比较、`&&`、`||`。
Operand、argument、construction operand 与 range endpoint 按源码顺序各求值一次；
`&&` 和 `||` 只在必要时求值右侧。

## 数值语义

类型严格匹配，没有隐式数值转换。Integer literal 使用 expected integer type，否则默认
`i32`；float literal 为 `f64`。仅提供精确的 `i32_to_f64` 与 `u32_to_f64` 转换。
没有 width cast、float-to-integer cast、`as` 或 constructor cast。浮点遵守严格 double
语义，不启用 fast-math，也不承诺跨 backend bit-identical。

Integer arithmetic 默认 unchecked。`--overflow checked` 是 C 与 Native backend mode，
通过 checked status contract 报告 overflow 和 division/modulo fault，不是源语法。

## Raw pointer 与 slice

Pointer index 接受 `i32`、`u32` 或 compatible integer literal，不做有效性或边界检查。

`slice(data, len)` 以 `ptr<T>` 和 `u32` length 构造 `slice<T>`。Memory validity、
alignment、allocation extent、lifetime 及 length 真实性由调用方负责；复制后仍 alias 同一内存。

`items[index]` 的 index 必须为 `u32`。`items[start..end]` 创建半开 sub-slice，合法执行
要求 `start <= end <= len`，等价写作 `start <= end <= items.len`。`.data` 返回 `ptr<T>`，`.len`
返回 `u32`。

C 与 Native 可用 `--bounds checked` 检查 slice index 和 sub-slice。Raw pointer index、
`slice(data, len)` 与通过 `.data` 的 index 永不由 CK 验证。Unchecked mode 与 WebAssembly
不生成 slice guard；WebAssembly 拒绝 checked mode。

## Diagnostic 与非目标

稳定 frontend code 见 [Diagnostic](diagnostics.md)。0.10 不提供 module/import、dynamic
allocation、ownership runtime、exception、async、closure、pointer/slice 之外的 source
generic、`f32`、SIMD source type、GPU target、program argument、stdin、thread 或公开
embeddable JIT API。
