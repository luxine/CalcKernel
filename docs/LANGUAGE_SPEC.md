# CalcKernel Language Specification

[简体中文](zh-CN/LANGUAGE_SPEC.md)

This document describes the CK / CalcKernel V0 language.

CK / CalcKernel is a high-performance DSL for pure computation kernels. It is
not a general purpose programming language. V0 is designed to compile `.ck`
source into C, WASM, and LLVM backend outputs for host languages and native
toolchains. Integer kernels remain the primary target; Phase 16 adds strict
`f64` for numerical kernels.

## Source Files

CalcKernel source files use the `.ck` extension.

## Supported Types

V0 supports these types:

- `i32`
- `i64`
- `u32`
- `u64`
- `f64`
- `bool`
- `ptr<T>`
- `struct`

`ptr<T>` represents a caller-owned pointer to `T`. V0 has no owned arrays and
no dynamic allocation.

`f64` is strict floating point. It is intended for numerical kernels. It is the
only floating point type in CK / CalcKernel; `f32` is not planned. It is not
recommended for money, tax, POS totals, or pricing-rule calculations; use `i64`
fixed-point arithmetic for those domains so checked integer mode can report
overflow and division errors explicitly.

The language does not support `f32`, implicit int/float conversion, fast-math,
SIMD, or float checked overflow. Phase 20 adds only exact explicit `i32`/`u32`
to `f64` casts through compiler builtins. Other cast directions remain
unsupported.

## Supported Declarations

### Structs

```ck
struct Item {
  price: i64;
  qty: i64;
}
```

Struct fields are named and typed. Duplicate struct names and duplicate fields
inside a struct are type errors.

### Functions

```ck
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

Functions have typed parameters and a typed return value. `export fn` is emitted
in the generated C header. Non-exported `fn` declarations are emitted as
`static` C functions and are not declared in the header.

## Supported Statements

- `let`
- assignment
- `return`
- `if` / `else`
- `while`
- block statements

Examples:

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

V0 functions must definitely return along the final statement path. A function
ending without a return is a type error.

## Supported Expressions

- integer literals
- float literals
- boolean literals: `true`, `false`
- variable references
- function calls
- unary operators: `!`, unary `-`
- arithmetic operators: `+`, `-`, `*`, `/`, `%`
- comparison operators: `==`, `!=`, `<`, `<=`, `>`, `>=`
- logical operators: `&&`, `||`
- pointer index access: `items[i]`
- struct field access: `item.price`
- combined access: `items[i].price`
- parentheses

### Float Literals

Float literals have type `f64`.

Supported forms:

- `1.0`
- `0.5`
- `1e3`
- `1.0e-3`
- `2E8`
- `2E+8`

Unsupported forms:

- `1.`
- `.5`
- `1e`
- `1e+`
- suffixes such as `1.0f64`
- underscores such as `1_000.0`
- `NaN` or `Inf` literal syntax

Negative numbers are parsed as unary `-` applied to a literal. For example,
`-1.0` is unary minus plus a `FloatLiteral`, not a signed literal token.

### Explicit int-to-f64 Casts

CK / CalcKernel supports two exact compiler builtins for crossing from 32-bit
integers into strict `f64` code:

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

Both conversions are exact because every `i32` and `u32` value is representable
as `f64`. They are compiler builtins, not runtime functions, and their names are
reserved.

The following remain unsupported:

- `i64_to_f64`
- `u64_to_f64`
- any `f64_to_*` cast
- overloaded `to_f64(x)`
- `x as f64`
- constructor-like `f64(x)`
- implicit conversions

`let x: f64 = 1` and `i32_value + 1.0` are still type errors. Use
`i32_to_f64(i32_value) + 1.0` when an explicit conversion is intended.
If an explicit cast result participates in division by zero, the result follows
ordinary strict f64 behavior and may be infinity or NaN; it is not a checked
integer error.

### f64 Edge Semantics

CK / CalcKernel f64 semantics are strict and intentionally narrow:

- `NaN` and infinity do not have literal syntax; they can only be produced by
  arithmetic such as `0.0 / 0.0` or `1.0 / 0.0`.
- `-0.0` is observable through backend floating point behavior and must not be
  optimized away by algebraic rewrites.
- NaN comparisons follow the selected backend's ordinary IEEE-like behavior;
  tests classify NaN with `isNaN` and do not compare NaN payloads.
- Infinity is classified by sign.
- Finite cross-backend e2e checks use absolute and relative tolerance.
- CK / CalcKernel does not guarantee bit-identical floating point results across
  C, LLVM, WASM, and JavaScript hosts.

## Operator Precedence

Operators are listed from highest precedence to lowest precedence.

| Precedence | Operators / forms | Associativity |
| --- | --- | --- |
| 1 | function call `f(...)`, index `a[i]`, field `a.b` | left |
| 2 | unary `!`, unary `-` | right |
| 3 | `*`, `/`, `%` | left |
| 4 | `+`, binary `-` | left |
| 5 | `<`, `<=`, `>`, `>=` | left |
| 6 | `==`, `!=` | left |
| 7 | `&&` | left |
| 8 | `||` | left |

Parentheses override the default precedence.

## Type Checking Rules

V0 type checking is intentionally strict:

- All variable references must resolve to a parameter or local variable.
- Function calls must resolve to a declared function.
- Function call argument count must match exactly.
- Function call argument types must be assignable to parameter types.
- Struct types must be declared before they can be used as named types.
- Struct field access requires a struct value and an existing field.
- Index access requires a pointer value.
- Pointer index expressions must be `i32`, `u32`, or an integer literal.
- `if` and `while` conditions must be `bool`.
- Assignment targets must be variables, fields, or index expressions.
- Assignment value type must be assignable to the target type.
- Return value type must be assignable to the function return type.
- Arithmetic operators require operands of the same numeric type.
- Integer arithmetic supports `+`, `-`, `*`, `/`, and `%`.
- f64 arithmetic supports `+`, `-`, `*`, and `/`.
- `f64 % f64` is rejected.
- Ordered comparisons require operands of the same numeric type.
- Equality comparisons require compatible operand types.
- Logical operators require `bool` operands and return `bool`.
- Unary `!` requires `bool` and returns `bool`.
- Unary `-` requires an integer or `f64` operand and returns the same type.
- Mixed integer/f64 arithmetic and comparisons are rejected.
- Integer literals do not materialize to `f64`.
- Float literals do not materialize to integer types.

Integer literals are materialized to the expected integer type when context is
available. Otherwise they default to `i32`.

Examples rejected by the type checker:

```ck
let x: f64 = 1;
let y: i64 = 1.0;
let z: f64 = 1.0 + 2;
let w: bool = 1.0 < 2;
```

## V0 Non-Goals

V0 does not support:

- strings
- IO
- dynamic memory allocation
- heap allocation
- garbage collection
- exceptions
- async
- classes or objects
- closures
- modules or imports
- runtime library
- checked overflow as a language syntax feature or default behavior
- bounds checks
- `f32` (not planned)
- `f64 %`
- implicit int/float conversion
- `i64_to_f64` / `u64_to_f64`
- f64-to-int casts
- broad or overloaded cast systems
- fast-math
- SIMD
- JIT compilation
- `NaN` literal syntax
- `Infinity` literal syntax
- float suffix literal syntax
- cross-backend bit-identical floating point guarantees

## Integer Overflow

V0 defaults to unchecked integer overflow. With `--overflow unchecked`, generated
C uses ordinary C integer operations for the mapped C type and does not insert
overflow or division-by-zero checks.

The compiler also supports optional checked arithmetic code generation with
`--overflow checked`. Checked mode changes the generated C ABI to return
`CK_Status`, writes the original CalcKernel return value through a final
`ck_return` pointer, and checks integer add, subtract, multiply, divide, modulo,
and unary minus operations. It also reports division by zero and signed
division/modulo overflow such as `INT64_MIN / -1`.

Checked arithmetic is a code generation mode, not new `.ck` syntax. It does not
add bounds checks, pointer validity checks, heap allocation, runtime support, or
exceptions.

`f64` arithmetic is not checked for overflow. In checked C mode, f64 operations
use ordinary strict C `double` behavior: f64 division by zero does not return
`CK_ERR_DIV_BY_ZERO`, and f64 overflow does not return `CK_ERR_OVERFLOW`. See
[Checked Arithmetic](CHECKED_ARITHMETIC.md) for the full ABI and safety
boundary.
