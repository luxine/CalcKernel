# `ckc` V0.9 CLI Reference

[简体中文](../zh-CN/reference/cli.md)

This document normatively describes the native `ckc` command surface. Input
files use `.ck`. Successful commands exit 0; usage, source, file-system,
unsupported-mode, toolchain, and backend failures exit nonzero and write their
error to stderr. Successful status text is written to stdout except debug/pass
printing, which is written to stderr.

## Commands and artifacts

| Command | Result |
| --- | --- |
| `ckc check <file>` | Lex, parse, and type-check; no artifact. |
| `ckc emit-mir <file>` | MIR on stdout or in `--out`. |
| `ckc emit-c <file> --out <file.c>` | C source and a sibling header, or explicit `--header`. |
| `ckc emit-wat <file> --out <file.wat>` | Textual WebAssembly. |
| `ckc emit-wasm <file> --out <file.wasm>` | WebAssembly binary. |
| `ckc emit-llvm <file> --out <file.ll>` | Textual LLVM IR. |
| `ckc build <file> --out <path>` | Native C dynamic library through `clang`. |
| `ckc build-llvm <file> --out <path>` | LLVM dynamic library or object through `clang`. |

`emit-mir`, `emit-c`, `emit-wat`, and `emit-llvm` may omit `--out` where the
command supports stdout. `emit-wasm`, `build`, and `build-llvm` require it.
Parent directories are created; file replacement uses the compiler's atomic
output path where applicable.

## Flags

- `--out <file>` or `-o <file>` selects output.
- `--header <file>` selects the C header path for `emit-c`.
- `--overflow <unchecked|checked>` defaults to `unchecked`.
- `--bounds <unchecked|checked>` defaults to `unchecked`.
- `--opt-level <0|1|2|3>` defaults to 0; `-O0` through `-O3` are aliases.
- `--target <triple>` sets the LLVM target triple for LLVM commands.
- `--kind <dynamic|object>` selects `build-llvm` output and defaults to `dynamic`.
- `--print-pass-pipeline`, `--print-mir-before-opt`, and
  `--print-mir-after-opt` write optimizer diagnostics to stderr.
- `--help` prints the complete usage surface.

Flags are validated when their command consumes them. Argument-error precedence,
unknown command handling, and exact help text are compatibility-tested.

## Backend mode matrix

| Backend | Overflow unchecked | Overflow checked | Bounds unchecked | Bounds checked |
| --- | --- | --- | --- | --- |
| C (`emit-c`, `build`) | accepted | accepted | accepted | accepted |
| WASM (`emit-wat`, `emit-wasm`) | accepted | rejected | accepted | rejected |
| LLVM (`emit-llvm`, `build-llvm`) | accepted | rejected | accepted | rejected |

`check` and `emit-mir` are backend-independent. Unsupported checked modes are
rejected before artifact creation. See the [C checked modes](../abi/modes.md)
for status ABI details.
