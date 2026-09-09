# CalcKernel 0.13 兼容策略

[English](../../project/compatibility.md)

本文是 `0.13.x` 的规范性兼容权威。

Patch release 保持 0.13.0 已接受 source 与 observable semantics、稳定 diagnostic
identifier/category、已记录 CLI name/flag/default、stdout/stderr class、semantic textual MIR、
public C/WASM/Native C ABI shape、checked first-error order、runtime diagnostic byte/status，以及
六个 release archive name 与 checksum sidecar。

Patch release 可以拒绝非法输入、改善 diagnostic prose、增加 opt-in command、修复 codegen，
也可以在全部已承诺边界不变时优化。Private Rust module、KIR text/schema、profile wire
format、fact/proof encoding、pass algorithm、private LLVM bridge ABI、cache entry、dispatch/
collection runtime、measurement 与未记录 compiler interface 不是 public contract。

## 从 0.12.0 迁移到 0.13.0

- 已接受的 0.12 source、language semantics、semantic MIR、diagnostic、checked behavior、
  runtime output 与 public Native C ABI 保持兼容。Native C ABI 保持 version 1，Runtime ABI
  保持 version 2。
- KIR 从 v2 升级到 v3。CK workload profile annotation、site mapping、O2
  `CkLateProfileLayout`、O3 PGO transaction、multiversion bundle 与 dispatch plan 都是
  private compiler facility，不是 source-language promise。
- PGO 通过 `ckc pgo build|merge|inspect`、`--pgo-generate` 与 `--pgo-use` 显式启用；普通
  command 保持 profile-free。Profile use 接受 O2/O3，specialization 与
  `--cpu multiversion` 要求 O3。
- Native build/inspection 的 `--cpu` 接受 `baseline|native|multiversion`。Multiversion 输出
  支持 executable/dynamic/static，拒绝 multiversion object；baseline/native single-version
  profile-use object 继续支持。
- `CKPART01`/`CKPROF01` schema 1 是 compiler-owned workload format；旧、identity 不匹配、
  损坏、partial 或 unknown profile 都 fail closed，且不能作为安全证据。
- Private LLVM bridge 从 ABI 3 升级到 ABI 4；Native cache 从 `CKCOBJ02` key/manifest
  schema 3 升级到 `CKCOBJ03` key/manifest schema 4，并绑定完整 named-object bundle。
  旧 cache/bridge/KIR/profile client fail closed，不改变 foreign-call signature。
- Generation 与 dispatch runtime 是 compiler-private；generation flush symbol 和隐藏 variant
  symbol 不扩展 Native C ABI 1 或 Runtime ABI 2。
- Auto-Tuning remains 0.14；indirect-call promotion、scalable KIR vector 与 adaptive JIT PGO
  也不属于 0.13。

0.12 可执行兼容历史保留在 `tests/fixtures/compatibility/v0_12/manifest.toml`，其已接受 source
由当前 compatibility target 继续编译。

## 从 0.11.0 迁移到 0.12.0

- 0.11 source、semantic MIR、diagnostic 与 public Native C ABI 保持兼容；Native C ABI 仍为
  1，Runtime ABI 仍为 2。
- KIR 从 v1 升级到 v2 并绑定 `KirTargetProfile`；fixed-vector KIR、specialization、unroll、
  SLP、Loop SIMD 与 transactional audit state 都是 private facility，C/WebAssembly 保持 scalar。
- Native inspection 增加 consumer 与 baseline/native CPU 选择；private LLVM bridge 从 ABI 2
  升到 3，cache 升到 `CKCOBJ02` schema 3，旧 0.11 private entry fail closed。

可执行历史保留在 `tests/fixtures/compatibility/v0_12/manifest.toml`。

## 从 0.10.0 迁移到 0.11.0

- 增加 `unsafe fn` contract、显式 `unsafe { ... }` call 与 diagnostic `CK2014`–`CK2016`，
  不给既有 safe source 增加 undefined behavior。
- 增加 `emit-kir`、fact/effect inspection、optimization explanation，以及 opt-in
  `--sanitize-contracts`/`CKR0007`；semantic `emit-mir` 保持稳定。
- Native C ABI 保持 1；private LLVM bridge 与 Runtime ABI 升级到 2，exported unsafe
  function 保持 public C ABI。

可执行历史保留在 `tests/fixtures/compatibility/v0_11/manifest.toml`。

## 从 0.9.0 迁移到 0.10.0

- Native `build` 从 external Clang 改为 pinned in-process LLVM/LLD，并在统一 Native C ABI 下
  增加 executable/dynamic/static/object kind。
- 增加 `run`、无参数 internal `main` 与七个 Native print builtin；`build-llvm` 成为
  deprecated compatibility alias。
- 增加 checked Native mode，退出 standalone textual LLVM export-shape promise；C/WASM
  继续拒绝 reachable Native print。

0.10.0 identity 与 fixture 保留在 `tests/fixtures/compatibility/v0_10/manifest.toml`。未来
`1.0.0` 开始长期稳定承诺；0.13 line 不声称 1.0 compatibility。
