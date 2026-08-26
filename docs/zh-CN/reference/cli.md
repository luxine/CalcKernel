# `ckc` V0.9 CLI 参考

[English](../../reference/cli.md)

本文档规范原生 `ckc` 命令表面。输入使用 `.ck`。成功命令退出 0；usage、source、
filesystem、unsupported-mode、toolchain 与 backend failure 非零退出并写 stderr。
成功状态文本写 stdout；debug/pass 信息写 stderr。

## 命令与 artifact

| 命令 | 结果 |
| --- | --- |
| `ckc check <file>` | Lex/parse/type-check，不生成 artifact。 |
| `ckc emit-mir <file>` | stdout 或 `--out` 中的 MIR。 |
| `ckc emit-c <file> --out <file.c>` | C source 与 sibling/显式 `--header`。 |
| `ckc emit-wat <file> --out <file.wat>` | WAT text。 |
| `ckc emit-wasm <file> --out <file.wasm>` | WASM binary。 |
| `ckc emit-llvm <file> --out <file.ll>` | LLVM IR text。 |
| `ckc build <file> --out <path>` | 通过 `clang` 构建 C dynamic library。 |
| `ckc build-llvm <file> --out <path>` | 通过 `clang` 构建 LLVM dynamic library/object。 |

`emit-mir`、`emit-c`、`emit-wat`、`emit-llvm` 在支持 stdout 时可省略 `--out`；
`emit-wasm`、`build`、`build-llvm` 必须提供。Compiler 会创建 parent directory，
并在适用处使用 atomic output path。

## Flag

- `--out <file>` / `-o <file>`；C 可用 `--header <file>`。
- `--overflow <unchecked|checked>` 与 `--bounds <unchecked|checked>`，默认 unchecked。
- `--opt-level <0|1|2|3>`，默认 0；`-O0`–`-O3` 是 alias。
- LLVM 命令使用 `--target <triple>`；`build-llvm` 使用
  `--kind <dynamic|object>`，默认 dynamic。
- `--print-pass-pipeline`、`--print-mir-before-opt`、
  `--print-mir-after-opt` 写 stderr。
- `--help` 打印完整 usage。

## Backend mode matrix

| Backend | Overflow unchecked | Overflow checked | Bounds unchecked | Bounds checked |
| --- | --- | --- | --- | --- |
| C (`emit-c`, `build`) | 接受 | 接受 | 接受 | 接受 |
| WASM (`emit-wat`, `emit-wasm`) | 接受 | 拒绝 | 接受 | 拒绝 |
| LLVM (`emit-llvm`, `build-llvm`) | 接受 | 拒绝 | 接受 | 拒绝 |

`check` 与 `emit-mir` 与 backend 无关。Unsupported checked mode 在创建 artifact
前被拒绝。Status ABI 见 [C checked mode](../abi/modes.md)。
