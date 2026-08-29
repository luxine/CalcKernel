# CalcKernel 0.11 Checked C and Native Modes

[简体中文](../zh-CN/abi/modes.md)

`--overflow unchecked|checked` and `--bounds unchecked|checked` are independent.
All four combinations are accepted by Native and C; WebAssembly accepts only
both unchecked. Either checked selection enables the module-wide status ABI.

```c
typedef int32_t CK_Status;
#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)
```

A checked non-void function returns `CK_Status` and appends `T* ck_return`; it
writes the result only on success. A null result pointer returns
`CK_ERR_NULL_POINTER`. A void function returns status without an output pointer.
Internal calls use the same mode and propagate the first error; success is `CK_OK`.

Checked integer add, subtract, multiply, negation, division, and modulo report
overflow where applicable. Division or modulo by zero has its dedicated code;
signed minimum divided by -1 is overflow. Floating operations and exact
32-bit-integer-to-f64 conversions do not create status errors.

Checked `slice<T>` indexing requires `index < len`; checked half-open sub-slicing
requires `start <= end <= len`. Evaluation and first error order is: non-void
result pointer, then source operands left-to-right, then nested-call/arithmetic
failure, then the bounds guard. Thus overflow before bounds is observable when
arithmetic computes an index.

For Native `run` and executable entry, the compiler-owned wrapper supplies a
valid checked result pointer. A status failure prints one fixed runtime
diagnostic to stderr and exits 240 through 243. Output failure is 244 and
abnormal child termination is 245. Libraries return `CK_Status` and do not
print or translate it to a process exit code.

`--sanitize-contracts` is a separate opt-in Native run/executable debugging
mode. It checks each unsafe function entry, including recursive entries, before
the body executes. Failure prints exactly `CKR0007: unsafe contract violation`
plus LF and exits 246. Ordinary O0–O3 compilation inserts no contract checks;
sanitization does not make a false trusted precondition defined. Sanitized
libraries use a private test-only path and are not a public ABI variant.

The stable runtime diagnostics are ASCII/UTF-8 and end in exactly one LF byte:

| ID | Exact message before LF | Process status |
| --- | --- | ---: |
| `CKR0001` | `CKR0001: integer overflow` | 240 |
| `CKR0002` | `CKR0002: integer division or modulo by zero` | 241 |
| `CKR0003` | `CKR0003: null checked result pointer` | 242 |
| `CKR0004` | `CKR0004: slice index or sub-slice out of bounds` | 243 |
| `CKR0005` | `CKR0005: standard output write failed` | 244 |
| `CKR0006` | `CKR0006: native child terminated abnormally` | 245 |
| `CKR0007` | `CKR0007: unsafe contract violation` | 246 |

Statuses 240–246 are reserved. `CKR0005` is attempted on stderr after stdout
fails, and failure of that write does not change 244. Only the `ckc run` parent
emits `CKR0006`; a standalone executable retains host signal/exception behavior.

Checked modes are not memory safety. Raw pointer operations, `slice(data, len)`,
`.data` indexing, output buffers, allocation extent, alignment, lifetime,
aliasing, and concurrency remain caller responsibilities; bounds checks trust
the declared slice length.
