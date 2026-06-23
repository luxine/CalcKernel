# Checked Arithmetic Design

[简体中文](zh-CN/CHECKED_ARITHMETIC.md)

## Goal

IntKernel V0 defaults to unchecked arithmetic. Phase 10 adds an optional checked
arithmetic code generation mode for kernels that need safer integer behavior.

Checked mode is intended for money, tax, discount, pricing-rule, and other
integer-heavy domains where overflow or division by zero must be reported
instead of silently becoming generated C behavior.

Checked arithmetic currently applies to the C backend (`emit-c` and `build`).
The Phase 12 WASM backend is unchecked-only: `emit-wat --overflow checked` and
`emit-wasm --overflow checked` fail with a clear diagnostic. Checked WASM
lowering is future work.

## CLI Design

Unchecked mode remains the default:

```sh
ikc emit-c input.ik --out build/input.c --header build/input.h
ikc build input.ik --out build/libinput
```

The explicit forms are:

```sh
ikc emit-c input.ik --out build/input.c --header build/input.h --overflow unchecked
ikc emit-c input.ik --out build/input.c --header build/input.h --overflow checked

ikc build input.ik --out build/libinput --overflow unchecked
ikc build input.ik --out build/libinput --overflow checked
```

Default:

```text
--overflow unchecked
```

## Unchecked Mode

Unchecked mode preserves the current V0 behavior:

- C ABI is unchanged.
- Expressions are emitted directly as C expressions.
- Integer overflow is not checked.
- Division by zero is not checked.
- It has the lowest overhead.
- The caller and DSL author are responsible for valid inputs.

Unchecked mode keeps the original C ABI. Generated C source snapshots may change
when the default backend changes, but header ABI snapshots should remain stable
unless an ABI change is intentional.

## Unchecked vs Checked

| Topic | `--overflow unchecked` | `--overflow checked` |
| --- | --- | --- |
| Default | Yes | No |
| C ABI | Original return type | `IK_Status` return plus final return pointer |
| Integer overflow | Not checked | Returns `IK_ERR_OVERFLOW` |
| Division by zero | Not checked | Returns `IK_ERR_DIV_BY_ZERO` |
| User pointers | Not checked | Not checked, except generated `ik_return` |
| Bounds checks | No | No |
| Runtime dependency | None | None |
| Performance | Fastest | Extra checks and branches |

## Checked Mode

Checked mode changes exported function ABI:

- exported functions return `IK_Status`
- the original return value is written through a final output pointer
- generated C returns early on overflow, division by zero, or a null checked
  return pointer
- generated C is self-contained
- no runtime library is required
- no exceptions are used
- `setjmp` / `longjmp` are not used

Checked mode is a code generation mode, not a new language feature.

All IntKernel functions use the checked ABI in checked mode. Exported functions
appear in the generated header with `IK_API`; non-exported functions are emitted
as `static IK_Status` helpers inside the generated `.c` file.

As of Phase 11, checked C generation is based on the MIR pipeline:

```text
Typed Program -> MIR lowering -> MIR validator -> MIR C backend
```

MIR represents ordinary typed arithmetic, calls, places, and control flow. The
checked MIR C backend inserts overflow guards, division checks, `IK_Status`
propagation, and return-pointer handling while preserving the checked ABI.

## Status Values

Checked headers define:

```c
typedef int32_t IK_Status;

#define IK_OK ((IK_Status)0)
#define IK_ERR_OVERFLOW ((IK_Status)1)
#define IK_ERR_DIV_BY_ZERO ((IK_Status)2)
#define IK_ERR_NULL_POINTER ((IK_Status)3)
```

- `IK_OK`: computation succeeded.
- `IK_ERR_OVERFLOW`: checked arithmetic detected overflow.
- `IK_ERR_DIV_BY_ZERO`: division or modulo divisor was zero.
- `IK_ERR_NULL_POINTER`: the generated checked return pointer `ik_return` was
  `NULL`.

## Checked ABI Example

IntKernel source:

```ik
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

Checked header:

```c
typedef int32_t IK_Status;

#define IK_OK ((IK_Status)0)
#define IK_ERR_OVERFLOW ((IK_Status)1)
#define IK_ERR_DIV_BY_ZERO ((IK_Status)2)
#define IK_ERR_NULL_POINTER ((IK_Status)3)

IK_API IK_Status add_i64(int64_t a, int64_t b, int64_t* ik_return);
```

Checked implementation:

```c
IK_Status add_i64(int64_t a, int64_t b, int64_t* ik_return) {
  if (ik_return == NULL) {
    return IK_ERR_NULL_POINTER;
  }

  int64_t ik_tmp0;
  if (__builtin_add_overflow(a, b, &ik_tmp0)) {
    return IK_ERR_OVERFLOW;
  }

  *ik_return = ik_tmp0;
  return IK_OK;
}
```

## Checked Operations

### `+`

- Perform checked addition.
- Signed and unsigned integer types are checked.
- Overflow returns `IK_ERR_OVERFLOW`.

### `-`

- Perform checked subtraction.
- Signed and unsigned integer types are checked.
- Overflow returns `IK_ERR_OVERFLOW`.

### `*`

- Perform checked multiplication.
- Signed and unsigned integer types are checked.
- Overflow returns `IK_ERR_OVERFLOW`.

### `/`

- If divisor is zero, return `IK_ERR_DIV_BY_ZERO`.
- For signed integers, `INT32_MIN / -1` and `INT64_MIN / -1` return
  `IK_ERR_OVERFLOW`.
- Unsigned division only needs the zero-divisor check.
- Otherwise perform normal division.

### `%`

- If divisor is zero, return `IK_ERR_DIV_BY_ZERO`.
- For signed integers, `INT32_MIN % -1` and `INT64_MIN % -1` return
  `IK_ERR_OVERFLOW`.
- Unsigned modulo only needs the zero-divisor check.
- Otherwise perform normal modulo.

### Unary `-`

- For signed integers, `-INT32_MIN` and `-INT64_MIN` return
  `IK_ERR_OVERFLOW`.
- For unsigned integers, unary minus is lowered as checked subtraction from
  zero, so any non-zero value overflows.
- Otherwise perform normal negation.

## Logical Operators

`&&` and `||` must preserve source-language short-circuit semantics.

Example:

```ik
a != 0 && b / a > 0
```

If `a == 0`, the right side must not be evaluated, so `b / a` must not trigger a
division-by-zero error.

Phase 11 MIR lowering represents `&&` and `||` as control flow. The checked MIR
C backend follows those MIR blocks, so the right-hand side is not emitted or
evaluated before the branch that decides whether it is needed.

## Function Calls

In checked mode, calling another IntKernel function uses the checked ABI:

- pass the original arguments
- pass the address of a temporary variable for the callee return value
- inspect the returned `IK_Status`
- if the status is not `IK_OK`, return it from the current function
- otherwise use the temporary value as the call expression result

Conceptually:

```c
int64_t ik_tmp0;
IK_Status ik_status0 = add_i64(a, b, &ik_tmp0);
if (ik_status0 != IK_OK) {
  return ik_status0;
}
```

Function arguments are checked expressions too. For example:

```ik
return add(a + 1, b * 2);
```

lowers by first checking `a + 1` and `b * 2`, then passing their temporary
values to `add`. If the callee returns any status other than `IK_OK`, the caller
returns that same status immediately.

In MIR, a call expression is an explicit `Call` instruction with a result
temporary. Checked C emission lowers that instruction to a checked ABI call,
passes `&temporary` as the final return pointer, checks the returned
`IK_Status`, and propagates any non-`IK_OK` status.

## Pointer, Index, and Field Access

Checked mode supports V0 pointer, index, and struct field access in generated C:

```ik
items[i].price
items[i].qty
out[i] = value;
```

The generated code evaluates index expressions through the checked expression
lowering path. If the index expression contains arithmetic, that arithmetic is
checked before the pointer access is emitted:

```ik
items[i + 1].price
```

In this example, `i + 1` can return `IK_ERR_OVERFLOW`.

Phase 10 still does not add bounds checking.

Reason:

- V0 has `ptr<T>` but no length-carrying pointer type.
- The compiler cannot reliably determine whether `items[i]` is in bounds.

Checked mode does not check:

- `items[i]` bounds
- `out[i]` bounds
- whether user-provided pointers are valid
- whether user-provided buffers are long enough for `len`

The caller is responsible for:

- passing valid pointers when the kernel reads or writes through `ptr<T>`
- ensuring every index used by the kernel is in bounds
- ensuring output buffers are large enough
- ensuring pointer lifetimes cover the full native call

Future bounds checking requires a language-level design such as `slice<T>` or
explicit pointer-plus-length metadata.

## Null Pointers

Phase 10 only checks the generated checked ABI return pointer, `ik_return`, for
`NULL`.

It does not automatically check every user `ptr<T>` parameter.

Reasons:

- APIs may allow a data pointer to be `NULL` when `len == 0`.
- V0 has no `in`, `out`, or `nonnull` annotations.
- Automatically checking all pointers would change user-visible semantics.

User pointer validity remains the caller's responsibility.

## Compiler Requirement

The checked mode implementation currently relies on Clang/GCC-style overflow
builtins:

- `__builtin_add_overflow`
- `__builtin_sub_overflow`
- `__builtin_mul_overflow`

The current project build path uses clang. If IntKernel later supports native
MSVC compilation without clang-compatible builtins, the backend should add a
portable fallback or MSVC-specific lowering for checked add, subtract, and
multiply.

Division, modulo, and unary minus checks can be emitted directly using C
comparisons against type-specific min values and divisors.

## Performance Impact

Checked mode is expected to be slower than unchecked mode. The overhead comes
from:

- overflow builtin calls or equivalent compiler-lowered checks
- division-by-zero branches
- signed division/modulo overflow branches
- `IK_Status` checks after IntKernel function calls
- extra temporaries and the final `ik_return` write

Use checked mode when correctness and explicit arithmetic failure reporting are
more important than maximum throughput, for example money, tax, discount, and
rules engines. Use unchecked mode for hot paths where the caller or earlier
validation has already proven that overflow and division by zero cannot happen.

## Limitations

Checked mode does not provide complete memory safety:

- no bounds check
- no runtime
- no heap allocation
- no exceptions
- no checked pointer lifetime
- no checked buffer length
- no checked user-provided output buffers
- checked mode changes the C ABI
- checked mode may be slower than unchecked mode

Checked arithmetic improves integer error reporting, but it does not make
pointer-based kernels memory safe.
