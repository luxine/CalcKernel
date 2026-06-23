# IntKernel Language Specification

This document describes the IntKernel V0 language.

IntKernel is a small DSL for pure integer computation. It is not a general
purpose programming language. V0 is designed to compile `.ik` source into
readable C source and header files, which can then be compiled into native
libraries for host languages.

## Source Files

IntKernel source files use the `.ik` extension.

## Supported Types

V0 supports only these types:

- `i32`
- `i64`
- `u32`
- `u64`
- `bool`
- `ptr<T>`
- `struct`

`ptr<T>` represents a caller-owned pointer to `T`. V0 has no owned arrays and
no dynamic allocation.

## Supported Declarations

### Structs

```ik
struct Item {
  price: i64;
  qty: i64;
}
```

Struct fields are named and typed. Duplicate struct names and duplicate fields
inside a struct are type errors.

### Functions

```ik
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

V0 functions must definitely return along the final statement path. A function
ending without a return is a type error.

## Supported Expressions

- integer literals
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
- Arithmetic operators require integer operands of the same type.
- Ordered comparisons require integer operands of the same type.
- Equality comparisons require compatible operand types.
- Logical operators require `bool` operands and return `bool`.
- Unary `!` requires `bool` and returns `bool`.
- Unary `-` requires an integer operand and returns the same integer type.

Integer literals are materialized to the expected integer type when context is
available. Otherwise they default to `i32`.

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
- LLVM backend
- WASM backend
- JIT compilation

## Integer Overflow

V0 defaults to unchecked integer overflow. With `--overflow unchecked`, generated
C uses ordinary C integer operations for the mapped C type and does not insert
overflow or division-by-zero checks.

The compiler also supports optional checked arithmetic code generation with
`--overflow checked`. Checked mode changes the generated C ABI to return
`IK_Status`, writes the original IntKernel return value through a final
`ik_return` pointer, and checks integer add, subtract, multiply, divide, modulo,
and unary minus operations. It also reports division by zero and signed
division/modulo overflow such as `INT64_MIN / -1`.

Checked arithmetic is a code generation mode, not new `.ik` syntax. It does not
add bounds checks, pointer validity checks, heap allocation, runtime support, or
exceptions. See [Checked Arithmetic](CHECKED_ARITHMETIC.md) for the full ABI and
safety boundary.
