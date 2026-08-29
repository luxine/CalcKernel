# CalcKernel 0.11 兼容性策略

[English](../../project/compatibility.md)

本文档是 `0.11.x` 的规范性 compatibility authority。

Patch release 保持 0.11.0 已接受 source/observable semantics、diagnostic ID/category、CLI
name/flag/default、stdout/stderr class、semantic textual MIR、public C/WASM/Native C ABI、
checked first-error、runtime diagnostic byte/status，以及六个 archive name/checksum sidecar。
Private Rust module、KIR text/fact/proof encoding、pass algorithm、private LLVM bridge ABI、
cache entry、measurement 与 undocumented interface 不是 public contract。

## 0.10.0 到 0.11 migration

- 新增 `unsafe fn` contract、显式 `unsafe { ... }` call 和 `CK2014`–`CK2016`；已有 safe
  0.10 source 仍是 safe source，不会因 optimizer guess 获得 UB。
- 新增 `emit-kir`、fact/effect/explanation inspection；KIR 不是稳定跨版本格式。
- `--sanitize-contracts` 只用于 Native run/executable 调试，并新增 private `CKR0007`/246；
  普通编译信任 precondition，不插入检查。
- C/WASM/Native 统一消费 verified KIR；stable semantic `emit-mir` 保持兼容。
- Public Native C ABI 保持 1；private LLVM bridge/runtime ABI 为 2，cache/codegen identity
  使用 KIR v1，因此不复用 0.10 object。
- Exported unsafe function 的 C ABI 不变，header 增加 normalized contract comment；foreign
  caller 承担 entry obligation。

`tests/fixtures/compatibility/v0_11/manifest.toml` 把每项变化映射到 executable evidence，
并继续编译冻结边界上的 0.10 fixture。

## 0.9.0 到 0.10 migration

历史变化包括：Native 从 external Clang 转到进程内 LLVM/LLD，增加四种 artifact kind 与统一
Native C ABI；新增 `run`、internal `main` 和 print builtin；`build-llvm` 成为 deprecated
alias；新增 Native checked mode，退出 standalone LLVM export-shape promise，并保持
`emit-c` source-only 与 C/WASM reachable-print rejection。

长期 stability commitment 从未来 `1.0.0` 开始；0.11 不声明 1.0 compatibility。
