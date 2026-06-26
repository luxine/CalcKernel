# Checked Arithmetic Design

[简体中文](zh-CN/CHECKED_ARITHMETIC.md)

## Goal

CalcKernel V0 defaults to unchecked arithmetic. Phase 10 adds an optional checked
arithmetic code generation mode for kernels that need safer integer behavior.

Checked mode is intended for money, tax, discount, pricing-rule, and other
integer-heavy domains where overflow or division by zero must be reported
instead of silently becoming generated C behavior.

Checked arithmetic currently applies to the C backend (`emit-c` and `build`).
The Phase 12 WASM backend is unchecked-only: `emit-wat --overflow checked` and
`emit-wasm --overflow checked` fail with a clear diagnostic. The Phase 13 LLVM
backend is also unchecked-only: `emit-llvm --overflow checked` and
`build-llvm --overflow checked` fail with a clear diagnostic. Checked WASM and
checked LLVM lowering are future work.

## CLI Design

Unchecked mode remains the default:

```sh
ckc emit-c input.ck --out build/input.c --header build/input.h
ckc build input.ck --out build/libinput
```

The explicit forms are:

```sh
ckc emit-c input.ck --out build/input.c --header build/input.h --overflow unchecked
ckc emit-c input.ck --out build/input.c --header build/input.h --overflow checked

ckc build input.ck --out build/libinput --overflow unchecked
ckc build input.ck --out build/libinput --overflow checked
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
| C ABI | Original return type | `CK_Status` return plus final return pointer |
| Integer overflow | Not checked | Returns `CK_ERR_OVERFLOW` |
| Division by zero | Not checked | Returns `CK_ERR_DIV_BY_ZERO` |
| `f64` overflow and division by zero | C `double` behavior | C `double` behavior, still returns `CK_OK` unless another checked error occurs |
| User pointers | Not checked | Not checked, except generated `ck_return` |
| Bounds checks | No | No |
| Runtime dependency | None | None |
| Performance | Fastest | Extra checks and branches |

## Checked Mode

Checked mode changes exported function ABI:

- exported functions return `CK_Status`
- the original return value is written through a final output pointer
- generated C returns early on overflow, division by zero, or a null checked
  return pointer
- generated C is self-contained
- no runtime library is required
- no exceptions are used
- `setjmp` / `longjmp` are not used

Checked mode is a code generation mode, not a new language feature.

All CalcKernel functions use the checked ABI in checked mode. Exported functions
appear in the generated header with `CK_API`; non-exported functions are emitted
as `static CK_Status` helpers inside the generated `.c` file.

As of Phase 11, checked C generation is based on the MIR pipeline:

```text
Typed Program -> MIR lowering -> MIR validator -> MIR C backend
```

MIR represents ordinary typed arithmetic, calls, places, and control flow. The
checked MIR C backend inserts overflow guards, division checks, `CK_Status`
propagation, and return-pointer handling while preserving the checked ABI.

## Status Values

Checked headers define:

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
```

- `CK_OK`: computation succeeded.
- `CK_ERR_OVERFLOW`: checked arithmetic detected overflow.
- `CK_ERR_DIV_BY_ZERO`: division or modulo divisor was zero.
- `CK_ERR_NULL_POINTER`: the generated checked return pointer `ck_return` was
  `NULL`.

## Checked ABI Example

CalcKernel source:

```ck
export fn add_i64(a: i64, b: i64) -> i64 {
  return a + b;
}
```

Checked header:

```c
typedef int32_t CK_Status;

#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)

CK_API CK_Status add_i64(int64_t a, int64_t b, int64_t* ck_return);
```

Checked implementation:

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

- Perform checked addition.
- Signed and unsigned integer types are checked.
- Overflow returns `CK_ERR_OVERFLOW`.

### `-`

- Perform checked subtraction.
- Signed and unsigned integer types are checked.
- Overflow returns `CK_ERR_OVERFLOW`.

### `*`

- Perform checked multiplication.
- Signed and unsigned integer types are checked.
- Overflow returns `CK_ERR_OVERFLOW`.

### `/`

- For integer operands, if divisor is zero, return `CK_ERR_DIV_BY_ZERO`.
- For signed integers, `INT32_MIN / -1` and `INT64_MIN / -1` return
  `CK_ERR_OVERFLOW`.
- Unsigned division only needs the zero-divisor check.
- For `f64`, perform normal C `double` division. `1.0 / 0.0` does not return
  `CK_ERR_DIV_BY_ZERO`.
- Otherwise perform normal division.

### `%`

- If divisor is zero, return `CK_ERR_DIV_BY_ZERO`.
- For signed integers, `INT32_MIN % -1` and `INT64_MIN % -1` return
  `CK_ERR_OVERFLOW`.
- Unsigned modulo only needs the zero-divisor check.
- Otherwise perform normal modulo.

### Unary `-`

- For signed integers, `-INT32_MIN` and `-INT64_MIN` return
  `CK_ERR_OVERFLOW`.
- For unsigned integers, unary minus is lowered as checked subtraction from
  zero, so any non-zero value overflows.
- For `f64`, perform normal C `double` negation.
- Otherwise perform normal negation.

### `f64`

Checked C mode is still an integer checked arithmetic mode. Functions containing
`f64` use the checked ABI when `--overflow checked` is selected, so they return
`CK_Status` and write the source-level return value through `ck_return`. The f64
operations themselves use ordinary strict C `double` behavior:

- `f64 +`, `-`, `*`, and `/` do not call integer overflow builtins.
- f64 division by zero does not return `CK_ERR_DIV_BY_ZERO`.
- f64 overflow does not return `CK_ERR_OVERFLOW`.
- `i32_to_f64` and `u32_to_f64` casts do not return `CK_ERR_OVERFLOW`; they are
  exact explicit conversions into ordinary strict f64 values.
- f64 NaN, infinity, and `-0.0` are ordinary floating point results, not checked
  arithmetic status values.
- `f64 %` is not a language operation and is rejected before C emission.

This boundary is intentional: checked mode protects integer arithmetic. It does
not turn floating point into a trapping or exact decimal arithmetic mode.
Money, tax, POS totals, and pricing rules should continue to use `i64`
fixed-point arithmetic when exact checked failure reporting is required.

## Logical Operators

`&&` and `||` must preserve source-language short-circuit semantics.

Example:

```ck
a != 0 && b / a > 0
```

If `a == 0`, the right side must not be evaluated, so `b / a` must not trigger a
division-by-zero error.

Phase 11 MIR lowering represents `&&` and `||` as control flow. The checked MIR
C backend follows those MIR blocks, so the right-hand side is not emitted or
evaluated before the branch that decides whether it is needed.

## Function Calls

In checked mode, calling another CalcKernel function uses the checked ABI:

- pass the original arguments
- pass the address of a temporary variable for the callee return value
- inspect the returned `CK_Status`
- if the status is not `CK_OK`, return it from the current function
- otherwise use the temporary value as the call expression result

Conceptually:

```c
int64_t ik_tmp0;
CK_Status ik_status0 = add_i64(a, b, &ik_tmp0);
if (ik_status0 != CK_OK) {
  return ik_status0;
}
```

Function arguments are checked expressions too. For example:

```ck
return add(a + 1, b * 2);
```

lowers by first checking `a + 1` and `b * 2`, then passing their temporary
values to `add`. If the callee returns any status other than `CK_OK`, the caller
returns that same status immediately.

In MIR, a call expression is an explicit `Call` instruction with a result
temporary. Checked C emission lowers that instruction to a checked ABI call,
passes `&temporary` as the final return pointer, checks the returned
`CK_Status`, and propagates any non-`CK_OK` status.

## Pointer, Index, and Field Access

Checked mode supports V0 pointer, index, and struct field access in generated C:

```ck
items[i].price
items[i].qty
out[i] = value;
```

The generated code evaluates index expressions through the checked expression
lowering path. If the index expression contains arithmetic, that arithmetic is
checked before the pointer access is emitted:

```ck
items[i + 1].price
```

In this example, `i + 1` can return `CK_ERR_OVERFLOW`.

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

Phase 10 only checks the generated checked ABI return pointer, `ck_return`, for
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

The current project build path uses clang. If CalcKernel later supports native
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
- `CK_Status` checks after CalcKernel function calls
- extra temporaries and the final `ck_return` write

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
