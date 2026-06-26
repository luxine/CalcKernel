# CalcKernel 迁移指南

## Phase 21 Rename

Phase 21 将项目从 legacy IK / IntKernel 命名迁移到标准 CK / CalcKernel
命名。这里出现的旧名称只作为迁移参考。

| 旧名称 | 新名称 |
| --- | --- |
| `IK` | `CK` |
| `IntKernel` | `CalcKernel` |
| `ikc` | `ckc` |
| `.ik` | `.ck` |
| `intkernel` | `calckernel` |
| `IK_API` | `CK_API` |
| `IK_BUILD_DLL` | `CK_BUILD_DLL` |
| `IK_Status` | `CK_Status` |
| `IK_OK` | `CK_OK` |
| `IK_ERR_OVERFLOW` | `CK_ERR_OVERFLOW` |
| `IK_ERR_DIV_BY_ZERO` | `CK_ERR_DIV_BY_ZERO` |
| `IK_ERR_NULL_POINTER` | `CK_ERR_NULL_POINTER` |

## 兼容策略

本次不保留 legacy CLI 命令、源码后缀或 C ABI 名称的兼容别名：

- 不保留 `ikc` 命令别名。
- CLI 不接受 `.ik` 源码文件。
- 不生成 `IK_*` C ABI typedef 或 macro。

Checked C ABI 的状态码数值保持不变：

- `CK_OK`：`0`
- `CK_ERR_OVERFLOW`：`1`
- `CK_ERR_DIV_BY_ZERO`：`2`
- `CK_ERR_NULL_POINTER`：`3`

调用方需要更新命令调用、源码文件后缀、生成 header include、FFI binding，
以及所有检查 C ABI status macro 的代码。
