# CalcKernel Rename Migration Guide

This guide documents the Phase 21 project rename from the legacy naming to the
canonical CK / CalcKernel naming. The legacy names appear here only as migration
references. This is a breaking rename.

Recommended release version: `v0.7.0`, assuming the previous Phase 20 `v0.6.0`
release has already been committed and tagged. Do not publish the renamed npm
package until a human confirms the target package name is available.

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

The `ikc` alias is not kept.

The `.ik` compatibility path is not kept.

The `IK_` C ABI compatibility alias is not kept.

Users should update source files, build scripts, package references, generated
C headers, and FFI bindings during the same migration window.

## User Migration Steps

1. Rename source files from `.ik` to `.ck`.
2. Update scripts and documentation from `ikc` to `ckc`.
3. Update package references from `intkernel` to `calckernel`.
4. Update project references from `IntKernel` to `CalcKernel`.
5. Update short project references from `IK` to `CK`.
6. Regenerate C headers and update FFI code from `IK_*` names to `CK_*` names.
7. Re-run package fresh-install smoke tests against `node_modules/.bin/ckc`.

## Package And Repository Notes

The npm package should move from `intkernel` to `calckernel` for the rename
release, but npm availability must be checked manually before publish.

The repository may be renamed from `IntKernel` to `CalcKernel`, but this phase
does not require automated remote repository operations.

## C ABI Notes

Generated checked C status numeric values are preserved while the public names
change:

| Name | Value |
| --- | ---: |
| `CK_OK` | `0` |
| `CK_ERR_OVERFLOW` | `1` |
| `CK_ERR_DIV_BY_ZERO` | `2` |
| `CK_ERR_NULL_POINTER` | `3` |

No `IK_` typedefs or macros are emitted as compatibility aliases. Rebuild
generated headers and update C, Python ctypes, Node FFI, and other host bindings
to use the `CK_` names.

## Release Order Recommendation

Finish and freeze the Phase 20 `v0.6.0` release before starting the breaking
rename release. After Phase 20 is tagged, run the Phase 21 rename validation,
package dry-run, and fresh-install smoke using `ckc` and `.ck` sources before
tagging `v0.7.0`.
