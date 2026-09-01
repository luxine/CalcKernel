# CalcKernel 0.12 Language Reference

[简体中文](../zh-CN/reference/language.md)

This document is the normative source-language contract for CalcKernel 0.12.
CK is a deterministic computation-kernel language; source files use `.ck`.

## Types and declarations

Value types are `i32`, `i64`, `u32`, `u64`, `f64`, `bool`, `ptr<T>`,
`slice<T>`, and named structs. `void` is return-only: `-> void` is valid, but it cannot be a parameter,
local, field, pointer or slice element, operand, or value. Direct
`slice<slice<T>>` elements and exported slice returns are invalid; internal
functions may return slices.

Pointers and slices refer to caller-owned memory. CK does not allocate, free,
retain, or extend its lifetime. A slice contains a typed data pointer followed
by a `u32` length. Struct fields retain declaration order.

Functions have typed parameters and one return type. `export fn` contributes a
Native, C, or WebAssembly library export; plain `fn` is internal. Names in one
declaration scope are unique and every called function and named type resolves.

## Entry and native output

`main` is reserved and has exactly one of these parameterless, non-exported
forms:

```ck
fn main() -> void
fn main() -> i32
```

`ckc run` and `ckc build --kind executable` require `main`. A void entry exits
with status 0; an i32 entry supplies the platform process status. Portable CK
programs use 0 through 239 because 240 through 245 are reserved runtime failure
statuses.

The following reserved compiler builtins are available only to the Native entry
and runtime-effect model:

- `print_i32(i32) -> void`, `print_i64(i64) -> void`
- `print_u32(u32) -> void`, `print_u64(u64) -> void`
- `print_f64(f64) -> void`, `print_bool(bool) -> void`
- `print_newline() -> void`

Arguments are evaluated once in source order. Native executable and `run`
roots may reach these calls. A reachable print from a Native library/object
export, a C artifact root, or a WebAssembly export is rejected. Unreachable
print code may be removed. CK 0.12 has no general strings or byte I/O.

Value prints do not append a newline; `print_newline` emits exactly one LF on
every platform. Integers are base 10 without locale, grouping, leading zero, or
positive sign. Booleans are `true` or `false`. Finite f64 uses a no-allocation
shortest-round-trip decimal under round-to-nearest, ties-to-even. Negative zero
is `-0.0`; special spellings are `nan`, `inf`, and `-inf`, without NaN payload
or sign. Every print is an ordered observable effect. Output failure terminates
the process with `CKR0005` rather than returning `CK_Status`.

## Statements and control flow

Statements are typed `let`, assignment, `return`, `if` / `else`, `while`,
`break;`, `continue;`, blocks, and void-returning call statements. `break;`
exits the innermost loop and `continue;` branches to its condition. Either
outside a loop is `CK2009`; code after a non-fallthrough statement in the same
block is `CK2010`. Invalid void use is `CK2011`; invalid slice shape or operation
is `CK2012`. Loops are conservatively considered able to exit.

A value-returning function returns a value on every final path. A void function
may fall through or use `return;`. Returning the wrong presence of value,
discarding a non-void call, or using a void call as a value is invalid.
Assignment targets are locals, parameters, fields, pointer indices, or slice
indices. Slice `.data` and `.len` projections are read-only; a whole descriptor
remains assignable.

## Expressions and evaluation

Expressions include literals, identifiers, calls, parentheses, unary `!` and
`-`, arithmetic, comparisons, short-circuit boolean operations, field access,
pointer/slice indexing, sub-slicing, and slice construction. Precedence from
high to low is postfix; unary; `* / %`; `+ -`; ordered comparisons; equality;
`&&`; then `||`. Binary operators associate left and unary operators right.

Operands, arguments, construction operands, and range endpoints are evaluated
once in source order. `&&` and `||` evaluate the right side only when needed.

## Numeric semantics

Typing is exact; there are no implicit numeric conversions. Integer literals
use an expected integer type or default to `i32`; floating literals are `f64`.
Integer types support `+ - * / %`; `f64` supports `+ - * /`.

The only conversions are `i32_to_f64(i32) -> f64` and
`u32_to_f64(u32) -> f64`. Both are exact. There are no width casts,
floating-to-integer casts, `as`, or constructor casts. Floating execution uses
strict double-precision semantics without fast-math; cross-backend bit identity
is not promised.

Unchecked integer arithmetic is the default. `--overflow checked` is a C and
Native backend mode that reports overflow and division/modulo faults through
the checked status contract. It is not source syntax.

## Unsafe functions and trusted contracts

New optimizer assumptions enter only through an `unsafe fn` with at least one
`requires` clause. Every call to such a function, including a recursive or
unsafe-to-unsafe call, must be inside an explicit `unsafe { ... }` statement.
`main` must remain safe and cannot have a contract or effects clause.

```ck
export unsafe fn saxpy(x: slice<f64>, y: slice<f64>, n: u32) -> void
contract {
  requires n <= x.len && n <= y.len;
  requires noalias(x, y);
  requires aligned(x.data, 32);
  effects read(x), write(y);
}
{
  // caller promises every requires clause at entry
}
```

Contract expressions are compile-time facts over mathematical integers. The
closed 0.12 language permits integer parameters/constants, `slice.len`, affine
`+`/`-` and multiplication by a constant, comparisons, conjunction,
`multiple_of(value, positive_constant)`, `noalias(slice, slice)`, and
`aligned(pointer, power_of_two)`. It excludes calls, loads, stores, disjunction,
negation, mutable state, target hints, local assumptions, and loop contracts.

The optional `effects` ceiling is `none` or a comma-separated set of
`read(slice)`, `write(slice)`, and `readwrite(slice)` over named slice
parameters. It bounds externally reachable memory, including transitive calls;
private local storage is excluded. Runtime print, possible checked failure,
unsafe-call presence, and an unclassifiable `readwrite all` effect are always
inferred and cannot be hidden. An incomplete ceiling is `CK2016`.

The caller is responsible for every `requires` clause whenever control enters
the function. A false requirement makes that execution undefined at O0 through
O3 on every backend; normal compilation inserts no checks. `--sanitize-contracts`
is an opt-in Native run/executable debugging mode, not a semantic change and
not a way to make an invalid call defined. Unsafe contracts do not change the
C ABI, and exported headers contain normalized contract comments.

## Raw pointers and slices

Pointer indexing accepts `i32`, `u32`, or a compatible integer literal and has
no CK validity or bounds check.

`slice(data, len)` constructs `slice<T>` from `ptr<T>` and a `u32` length.
Memory validity, alignment, allocation extent, lifetime, and the truth of the
declared length remain the caller's responsibility. Copies alias the same
memory.

`items[index]` requires `u32`. `items[start..end]` creates a half-open
sub-slice; valid execution requires `start <= end <= len`, equivalently
`start <= end <= items.len`. `.data` yields `ptr<T>` and `.len` yields `u32`.

C and Native support optional `--bounds checked` guards for slice indexing and
sub-slicing. Raw pointer indexing, `slice(data, len)`, and indexing through
`.data` are never validated. Unchecked modes and WebAssembly emit no slice
guards; WebAssembly rejects checked modes.

## Diagnostics and non-goals

Stable frontend codes are listed in [Diagnostics](diagnostics.md). Source
typing is backend-independent. CalcKernel 0.12 has no modules/imports, dynamic
allocation, ownership runtime, exceptions, async, closures, source generics
beyond pointer/slice constructors, `f32`, SIMD source types, GPU target,
program arguments, stdin, threads, or public embeddable JIT API.
