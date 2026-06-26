# CalcKernel Rename 迁移指南

本文档记录 Phase 21 项目 rename：从 legacy naming 迁移到标准 CK /
CalcKernel naming。旧名称只在本文档中作为迁移参考出现。这是 breaking rename。

推荐 release version：`v0.7.0`，前提是 Phase 20 `v0.6.0` 已经完成 commit 和
tag。发布新的 npm package 前，必须由人工确认目标 package name 可用。

## Mapping

```text
IK -> CK
IntKernel -> CalcKernel
ikc -> ckc
.ik -> .ck
intkernel -> calckernel
IK_API -> CK_API
IK_BUILD_DLL -> CK_BUILD_DLL
IK_Status -> CK_Status
IK_OK -> CK_OK
IK_ERR_OVERFLOW -> CK_ERR_OVERFLOW
IK_ERR_DIV_BY_ZERO -> CK_ERR_DIV_BY_ZERO
IK_ERR_NULL_POINTER -> CK_ERR_NULL_POINTER
```

## Compatibility Policy

不保留 `ikc` alias。

不保留 `.ik` compatibility path。

不保留 `IK_` C ABI compatibility alias。

用户应在同一个迁移窗口中更新源码文件、构建脚本、package reference、生成的 C
header 和 FFI binding。

## 用户迁移步骤

1. 将源码文件从 `.ik` 重命名为 `.ck`。
2. 将脚本和文档中的 `ikc` 更新为 `ckc`。
3. 将 package reference 从 `intkernel` 更新为 `calckernel`。
4. 将项目名 reference 从 `IntKernel` 更新为 `CalcKernel`。
5. 将短项目名 reference 从 `IK` 更新为 `CK`。
6. 重新生成 C header，并将 FFI 代码中的 `IK_*` 名称更新为 `CK_*`。
7. 使用 `node_modules/.bin/ckc` 重新运行 package fresh-install smoke。

## Package And Repository Notes

rename release 应将 npm package 从 `intkernel` 迁移到 `calckernel`，但 npm
availability 必须在 publish 前人工确认。

repository 可以从 `IntKernel` 迁移到 `CalcKernel`，但本阶段不自动操作远程仓库。

## C ABI Notes

Generated checked C status 的 numeric value 保持不变，只改变公开名称：

| Name | Value |
| --- | ---: |
| `CK_OK` | `0` |
| `CK_ERR_OVERFLOW` | `1` |
| `CK_ERR_DIV_BY_ZERO` | `2` |
| `CK_ERR_NULL_POINTER` | `3` |

不会生成 `IK_` typedef 或 macro 作为 compatibility alias。需要重新生成 header，
并将 C、Python ctypes、Node FFI 和其他 host binding 更新到 `CK_` 名称。

## Release Order Recommendation

建议先完成并冻结 Phase 20 `v0.6.0` release，再开始 breaking rename release。
Phase 20 tag 完成后，再使用 `ckc` 和 `.ck` source 运行 Phase 21 rename
validation、package dry-run 和 fresh-install smoke，然后 tag `v0.7.0`。
