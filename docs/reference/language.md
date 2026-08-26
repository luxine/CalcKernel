# CK V0.9 Language Reference

[简体中文](../zh-CN/reference/language.md)

This document is the normative source-language contract for CalcKernel 0.9.
CK is a deterministic computation-kernel language. Source files use `.ck`.

## Types and declarations

The value types are `i32`, `i64`, `u32`, `u64`, `f64`, `bool`, `ptr<T>`,
`slice<T>`, and named structs. `void` is a return-only type: `-> void` is valid
but it is never a parameter, local, field, pointer/slice element, operand, or
value. Direct `slice<slice<T>>` elements and exported slice returns are invalid;
internal functions may return slices.

`ptr<T>` and `slice<T>` refer to caller-owned memory. CK does not allocate,
free, retain, or extend the lifetime of that memory. A slice is a descriptor
containing a typed data pointer and a `u32` length.

Structs contain ordered, named, typed fields:

```ck
struct Item {
  value: i32;
}
```

Functions declare typed parameters and one return type. `export fn` contributes
an externally callable backend symbol; plain `fn` is internal.

```ck
export fn add(a: i64, b: i64) -> i64 {
  return a + b;
}

fn touch(items: slice<Item>) -> void {
  return;
}
```

Names in the same declaration scope must be unique. Named struct types and
called functions must resolve.

## Statements and control flow

Supported statements are typed `let`, assignment, `return`, `if` / `else`,
`while`, `break;`, `continue;`, a block, and a call statement whose callee
returns `void`.

`break;` exits the innermost `while`; `continue;` jumps to that loop's condition.
Either outside a loop is `CK2009`. A statement after a non-fallthrough statement
in the same block is `CK2010`. Invalid void positions use `CK2011`; invalid slice
shapes and operations use `CK2012`. A loop is conservatively treated as able to exit,
even when its condition is the literal `true`.

A value-returning function must return a value on every final path. A void
function may fall through or use `return;`. Returning a value from a void
function, omitting a value in a non-void function, discarding a non-void call,
or using a void call as a value is invalid.

Assignment targets are locals, parameters, fields, pointer indices, or slice
indices. Slice `.data` and `.len` projections are read-only; a whole slice
descriptor remains assignable.

## Expressions

Expressions include integer, `f64`, and boolean literals; identifiers; calls;
parentheses; unary `!` and `-`; arithmetic `+ - * / %`; comparisons
`== != < <= > >=`; short-circuit `&&` and `||`; field access; pointer/slice
indexing; sub-slicing; and slice construction.

Precedence, highest to lowest, is:

| Level | Forms | Associativity |
| --- | --- | --- |
| 1 | call, index/sub-slice, field | left |
| 2 | unary `!`, unary `-` | right |
| 3 | `*`, `/`, `%` | left |
| 4 | `+`, binary `-` | left |
| 5 | `<`, `<=`, `>`, `>=` | left |
| 6 | `==`, `!=` | left |
| 7 | `&&` | left |
| 8 | `||` | left |

Operands, call arguments, slice construction operands, and range endpoints are
evaluated once in source order. `&&` and `||` evaluate the right operand only
when required.

## Strict typing and numeric semantics

Operators require exact compatible types; there are no implicit numeric
conversions. Integer literals materialize to an expected integer type or
default to `i32`. Float literals have type `f64`. Integer arithmetic supports
`+ - * / %`; `f64` supports `+ - * /` but not `%`.

The only conversions are reserved compiler builtins:

- `i32_to_f64(i32) -> f64`
- `u32_to_f64(u32) -> f64`

Both are exact. There are no integer-width casts, `f64`-to-integer casts,
implicit casts, `as`, or constructor-style casts.

Float literals require digits on both sides of a decimal point when a point is
present; exponent notation is supported. `NaN` and infinity have no literal
syntax but may result from arithmetic. Backends preserve ordinary strict
double-precision behavior, including signed zero and unordered NaN comparisons;
bit-identical cross-backend floating-point results are not promised.

Unchecked integer code generation is the default. `--overflow checked` is a C
backend mode that reports integer overflow and integer division/modulo errors;
it is not source syntax and does not check floating-point operations.

## Raw pointers and slices

Pointer indexing accepts `i32`, `u32`, or a context-compatible integer literal.
It has no CK validity or bounds check.

`slice(data, len)` constructs `slice<T>` from `ptr<T>` and a `u32` length.
Memory validity, alignment, allocation extent, lifetime, and the truth of the
declared length remain the caller's responsibility. Copying a slice aliases the
same memory.

`items[index]` requires a `u32` index. `items[start..end]` creates a half-open
sub-slice; both endpoints are `u32` and valid execution requires
`start <= end <= items.len`. `.data` returns `ptr<T>` and `.len` returns `u32`.

Slice bounds checking exists only in generated C when `--bounds checked` is selected.
Raw pointer indexing, slice construction, and indexing through `.data` are never validated by CK.
Unchecked C, WASM, and LLVM emit no slice guards. WASM and LLVM reject checked
bounds mode.

## Diagnostics and non-goals

Lexing, parsing, and type checking report stable codes described in
[Diagnostics](diagnostics.md). Strict source typing is independent of backend
selection.

V0.9 has no strings, I/O, modules/imports, dynamic allocation, ownership
runtime, exceptions, async, classes, closures, generics beyond `ptr<T>` and
`slice<T>`, `f32`, SIMD, GPU target, JIT, or source-level checked operators.
