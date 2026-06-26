# CalcKernel Migration Guide

## Phase 21 Rename

Phase 21 renames the project from the legacy IK / IntKernel identity to the
canonical CK / CalcKernel identity. These legacy names are documented here only
as migration references.

| Legacy name | New name |
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

## Compatibility Policy

No compatibility aliases are kept for the legacy CLI command, source extension,
or C ABI names:

- `ikc` is not retained as a command alias.
- `.ik` source files are not accepted by the CLI.
- `IK_*` C ABI typedefs and macros are not emitted.

The checked C ABI status values are preserved:

- `CK_OK`: `0`
- `CK_ERR_OVERFLOW`: `1`
- `CK_ERR_DIV_BY_ZERO`: `2`
- `CK_ERR_NULL_POINTER`: `3`

Consumers should update command invocations, source file suffixes, generated
header includes, FFI bindings, and any code that checks C ABI status macros.
