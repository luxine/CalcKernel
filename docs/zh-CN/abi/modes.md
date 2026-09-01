# CalcKernel 0.13 C 与 Native Checked Mode

[English](../../abi/modes.md)

`--overflow unchecked|checked` 与 `--bounds unchecked|checked` 相互独立。C 与 Native 接受
四种组合；WebAssembly 只接受两者均 unchecked。任一 checked selection 启用 module-wide
status ABI：

```c
typedef int32_t CK_Status;
#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)
```

Checked non-void function 返回 `CK_Status` 并追加 `T* ck_return`，只在成功时写 result；null
result pointer 返回 `CK_ERR_NULL_POINTER`。Void function 只返回 status。Internal call 使用
相同 mode 并传播 first error；成功为 `CK_OK`。

Checked integer arithmetic 在适用时报告 overflow；除零/模零使用独立 code；signed minimum
除以 -1 为 overflow。Checked slice index 要求 `index < len`，半开 sub-slice 要求
`start <= end <= len`。求值及 first error order 为 result pointer、source operand 从左到右、
nested-call/arithmetic failure、bounds guard；因此计算 index 时 overflow before bounds 可观察。

这里的 slice index 指 `slice<T>`，不是 raw pointer index。

Native `run`/executable entry wrapper 提供有效 result pointer。Status failure 写一条固定 stderr
diagnostic 并退出 240–243，output failure 为 244，abnormal child termination 为 245。Library
返回 `CK_Status`，不会打印或翻译为 process exit。

`--sanitize-contracts` 是独立 opt-in Native run/executable debug mode，在 body 执行前检查
每个 unsafe function entry（包括 recursive entry）。失败精确输出
`CKR0007: unsafe contract violation` 加 LF 并退出 246。普通 O0–O3 不插入 contract check，
sanitizer 也不会把 false trusted precondition 变成 defined execution。

稳定 runtime diagnostic 使用 ASCII/UTF-8，并以精确一个 LF byte 结束：

| ID | LF 前的精确 message | Process status |
| --- | --- | ---: |
| `CKR0001` | `CKR0001: integer overflow` | 240 |
| `CKR0002` | `CKR0002: integer division or modulo by zero` | 241 |
| `CKR0003` | `CKR0003: null checked result pointer` | 242 |
| `CKR0004` | `CKR0004: slice index or sub-slice out of bounds` | 243 |
| `CKR0005` | `CKR0005: standard output write failed` | 244 |
| `CKR0006` | `CKR0006: native child terminated abnormally` | 245 |
| `CKR0007` | `CKR0007: unsafe contract violation` | 246 |

240–246 为 reserved。Stdout 失败后会尝试把 `CKR0005` 写入 stderr，该写入再失败也不改变
244。只有 `ckc run` parent 输出 `CKR0006`；standalone executable 保留 host signal/exception
behavior。

Checked mode 不等于 memory safety。Raw pointer、`slice(data, len)`、`.data` index、output
buffer、allocation extent、alignment、lifetime、alias 与 concurrency 仍由 caller 负责。
