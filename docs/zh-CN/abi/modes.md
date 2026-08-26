# CalcKernel V0.9 Checked C Mode

[English](../../abi/modes.md)

本文档规范彼此独立的 C codegen option：`--overflow unchecked|checked` 与
`--bounds unchecked|checked`。`emit-c` / `build` 接受四种组合；WASM 与 LLVM
拒绝任一 checked mode。

| Overflow | Bounds | C ABI | 检查 |
| --- | --- | --- | --- |
| unchecked | unchecked | direct return / C `void` | 无 |
| checked | unchecked | status ABI | integer arithmetic 与 division/modulo |
| unchecked | checked | status ABI | slice index 与 sub-slice |
| checked | checked | status ABI | 两类 |

任一 checked selection 都启用完整 module-wide status ABI：

```c
typedef int32_t CK_Status;
#define CK_OK ((CK_Status)0)
#define CK_ERR_OVERFLOW ((CK_Status)1)
#define CK_ERR_DIV_BY_ZERO ((CK_Status)2)
#define CK_ERR_NULL_POINTER ((CK_Status)3)
#define CK_ERR_OUT_OF_BOUNDS ((CK_Status)4)
```

非 void function 返回 `CK_Status` 并追加 `T* ck_return`，只在成功时写 source
return value；null `ck_return` 返回 `CK_ERR_NULL_POINTER`。Void function 没有结果
pointer，显式/自然成功返回 `CK_OK`。Internal call 立即传播非 `CK_OK` status。

Checked integer add/subtract/multiply/unary negation/divide/modulo 在适用时报告
overflow；divide/modulo by zero 返回 `CK_ERR_DIV_BY_ZERO`；signed minimum 除以
`-1` 返回 overflow。`f64` operation 与 32-bit integer-to-f64 cast 不产生 status。

Checked `slice<T>` index 要求 `index < len`；半开 sub-slice 要求
`start <= end <= len`，失败在 pointer advance/access 前返回
`CK_ERR_OUT_OF_BOUNDS`。

可观察 error order：非 void null result pointer 最先；之后 source operand 按从左
到右各求值一次；计算 index/range 时的 nested call 或 arithmetic failure 先于其
bounds guard。Raw pointer indexing、`slice(data, len)`、通过 `.data` indexing、
output buffer、allocation extent、alignment、lifetime、aliasing 与 concurrency
仍由 caller 负责；bounds mode 信任 descriptor 声明长度。
