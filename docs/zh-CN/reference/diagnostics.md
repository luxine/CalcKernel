# CK V0.9 Diagnostic

[English](../../reference/diagnostics.md)

本文档规范 diagnostic ID。`0.9.x` 可以改善可读 message、source excerpt 与 caret
宽度，但不可改变 code 的阶段与含义。格式为
`file:line:column: error CKxxxx: message`，之后是源码行与 caret。

| Code | 阶段 | 触发条件 |
| --- | --- | --- |
| `CK0001` | Lexing | 非法 character 或 malformed numeric token。 |
| `CK1001` | Parsing | Token sequence 不符合 CK grammar。 |
| `CK2001` | Type checking | Unknown variable。 |
| `CK2002` | Type checking | Unknown function。 |
| `CK2003` | Type checking | Unknown named type。 |
| `CK2004` | Type checking | 未分配更窄 code 的一般 type/operator/argument/field/index/return semantic error。 |
| `CK2005` | Type checking | Duplicate declaration、parameter、local、struct 或 field。 |
| `CK2006` | Type checking | `if` 或 `while` condition 不是 `bool`。 |
| `CK2007` | Type checking | Invalid assignment target。 |
| `CK2008` | Type checking | 返回 value 的路径缺少 return。 |
| `CK2009` | Type checking | `break` / `continue` 不在 `while` 中。 |
| `CK2010` | Type checking | 同一 block 中 terminating statement 之后出现 unreachable statement。 |
| `CK2011` | Type checking | 非法 `void` position 或 void/value return mismatch。 |
| `CK2012` | Type checking | 非法 slice element、构造、projection、index/range、赋值/call/return shape 或 exported slice return。 |

同一源码可按确定的 source order 报告多个 diagnostic。任一 error 都使进程非零退出。
Backend/toolchain failure 属于 CLI error，不产生新的 `CKxxxx` semantic code。
