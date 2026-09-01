# CalcKernel 0.12 兼容策略

[English](../../project/compatibility.md)

本文是 `0.12.x` 的规范性兼容权威。

Patch release 保持 0.12.0 已接受 source 与 observable semantics、稳定 diagnostic
identifier/category、已记录 CLI name/flag/default、stdout/stderr class、semantic textual MIR、
public C/WASM/Native C ABI shape、checked first-error order、runtime diagnostic byte/status，以及
六个 release archive name 与 checksum sidecar。

Patch release 可以拒绝非法输入、改善 diagnostic prose、增加 opt-in command、修复 codegen，
也可以在全部已承诺语义边界不变时优化。Private Rust module、KIR text、fact/proof encoding、
pass algorithm、private LLVM bridge ABI、cache entry、measurement 与未记录的 compiler interface
不是 public contract。

## 从 0.11.0 迁移到 0.12.0

- 已接受的 0.11 source、language semantics、semantic MIR、diagnostic 与 public Native C ABI
  保持兼容。Native C ABI 保持 version 1，contract-aware runtime ABI 保持 version 2。
- KIR 从 v1 升级到 v2。每个 module 绑定规范化 `KirTargetProfile`；fixed-vector KIR、
  specialization、unroll、SLP、Loop SIMD、独立 checker 与 transactional optimizer audit state
  都是 private compiler facility，不新增 source vector language 或 public KIR ABI。
- `emit-kir --consumer inspection|c|wasm|native-library|native-executable` 选择精确 inspection
  profile。Native consumer 还接受 `--cpu baseline|native`；默认 inspection profile 仍为
  scalar、target-independent。
- C/WebAssembly 在 0.12 保持 scalar。Native 只有在 legality、strict semantics、cost、proof
  与 budget check 全部闭合时才自动生成 fixed-width SIMD。Checked/sanitizer 行为与可观察
  fallback 语义不变。
- Private LLVM bridge ABI 从 2 升级到 3。Native object/run cache 升级到 `CKCOBJ02`
  manifest schema 3，并包含 target-profile、proof/cost schema 与 optimizer-budget identity。
  0.11 cache entry 与旧 bridge client fail closed；foreign-call signature 不变。
- PGO remains 0.13。Auto-Tuning remains 0.14；0.12 不声称实现二者。

每项 0.12 有意变化都映射到
`tests/fixtures/compatibility/v0_12/manifest.toml` 的可执行证据；已接受的 0.11 fixture 在冻结
边界继续编译。

## 从 0.10.0 迁移到 0.11.0

- 增加 `unsafe fn` contract、显式 `unsafe { ... }` call 与 diagnostic `CK2014`–`CK2016`；
  既有 safe 0.10 source 仍为 safe source，不获得 optimizer-assumed undefined behavior。
- 增加 `emit-kir`、`--print-facts`、`--print-effect-summaries` 与
  `--explain-optimization` 的确定性 inspection；KIR v1 为 private。
- 增加 `--sanitize-contracts` Native run/executable opt-in 检查及 `CKR0007`/status 246；普通
  编译仍信任 unsafe precondition，不插入检查。
- C、WebAssembly 与 Native 开始消费同一 verified fact-driven KIR optimizer；稳定 semantic
  `emit-mir` 保持兼容。
- Native C ABI 保持 version 1；private LLVM bridge 与 contract-aware runtime ABI 升级到 2。
  Exported unsafe function 保持 C ABI，generated header 增加 normalized contract comment。

可执行历史保留在 `tests/fixtures/compatibility/v0_11/manifest.toml`。

## 从 0.9.0 迁移到 0.10.0

- Native `build` 从 external Clang 改为 pinned in-process LLVM/LLD，并在统一 Native C ABI 下
  增加 executable、dynamic、static、object kind。
- 增加 `run`、无参数 internal `main` 与七个 Native print builtin；其名称变成保留名。
- `build-llvm` 变为 deprecated compatibility alias，增加 checked Native mode，并退出独立
  textual LLVM export-shape promise。
- Native 不再留下 `.c`/`.ll` intermediate；`emit-c` 保持 source-only；C/WebAssembly 继续
  拒绝 reachable Native print。

未来 `1.0.0` 开始长期稳定承诺；0.12 line 不声称具备 1.0 compatibility。
