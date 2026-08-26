# CalcKernel V0.9 Checked C Modes

[简体中文](../zh-CN/abi/modes.md)

This document normatively defines the independent C code-generation options
`--overflow unchecked|checked` and `--bounds unchecked|checked`. All four
combinations are accepted by `emit-c` and `build`. WASM and LLVM reject either
checked mode.

## Mode matrix

| Overflow | Bounds | C ABI | Inserted checks |
| --- | --- | --- | --- |
| unchecked | unchecked | direct return / C `void` | none |
| checked | unchecked | status ABI | integer arithmetic and integer division/modulo |
| unchecked | checked | status ABI | slice index and sub-slice |
| checked | checked | status ABI | both sets |

Either checked selection activates the full module-wide status ABI:

```c
typedef int32_t CK_Status;
#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)
```

A non-void function returns `CK_Status` and appends `T* ck_return`; it writes the
source return value only on success. Null `ck_return` yields
`CK_ERR_NULL_POINTER`. A void function returns `CK_Status` without a result
pointer and returns `CK_OK` on explicit or natural success. Internal calls use
the same mode and immediately propagate non-`CK_OK` status.

## Checked operations and order

Integer add, subtract, multiply, unary negation, divide, and modulo report
overflow where applicable. Integer divide/modulo by zero reports
`CK_ERR_DIV_BY_ZERO`; signed minimum divided by `-1` reports overflow. Unsigned
arithmetic follows its selected checked/unchecked mode. `f64` operations and the
exact 32-bit integer-to-`f64` casts never produce status errors.

Checked `slice<T>` indexing requires `index < len`. Checked half-open sub-slicing
requires `start <= end <= len`; failure returns `CK_ERR_OUT_OF_BOUNDS` before
pointer advance or element access.

Observable error order is: a non-void null result pointer is checked first;
source operands are then evaluated once left-to-right; a nested call or
arithmetic failure while computing an index/range is propagated before its
bounds guard. In short: result-pointer failure first, then arithmetic before bounds
when arithmetic computes the checked access: overflow before bounds.

## Safety boundary

Checked modes do not establish memory safety. Raw pointer indexing,
`slice(data, len)`, indexing through `.data`, user-provided output buffers,
allocation extent, alignment, lifetime, aliasing, and concurrency remain the
caller's responsibility. Bounds mode trusts the descriptor's declared length.
