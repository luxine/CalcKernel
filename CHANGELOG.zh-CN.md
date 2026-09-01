# 变更日志

这里记录 CalcKernel 面向用户的重要变更。

## 0.12.0 - 尚未发布

- 新增 KIR v2 fixed-vector/mask instruction，以及由固定 LLVM 22.1.8 导出的确定性
  Native `KirTargetProfile` capability/cost 数据。
- 新增 transactional、由独立 checker 验证的 O3 specialization、受控 full/partial
  unroll、SLP 与 Loop SIMD frontier，并提供单调 analysis budget 和稳定 optimization
  explanation。
- 新增 integer/strict element-wise f64 arithmetic、supported cast、pure compare/select
  diamond、splat 与 contiguous memory 的 unit-stride Loop SIMD；strict f64 保持逐元素
  顺序且不启用 fast math。
- 新增以原 scalar loop 为 fallback 的 total runtime alias versioning、scalar epilogue，
  以及精确 unchecked modular u32 add/multiply reduction。Checked failure、effect、
  unsupported recurrence/scan、C 与 WebAssembly 仍保持 scalar。
- Private LLVM bridge 更新到 ABI 3，KIR identity 更新为 `kir-v2`，Native object cache
  更新为 `CKCOBJ02` key/manifest schema 3；public Native C ABI 1、Runtime ABI 2、source
  syntax、diagnostic 与 checked first-error behavior 保持不变。
- 新增 KIR/pre-LLVM/object structural evidence、fixed-seed O0/O3 differential、mutation、
  target-feature containment 与 schema-7 performance gate 输入。PGO/multiversioning 和
  Auto-Tuning 仍是未来工作。

## 0.11.0 - 尚未发布

- 新增显式 `unsafe fn` contract，支持 affine range requirement、`multiple_of`、
  `noalias`、alignment 与 slice memory-effect ceiling；unsafe call 必须位于
  `unsafe { ... }` statement 中，executable `main` 仍必须为 safe。
- 新增 deterministic `emit-kir` inspection、verified fact/effect summary、携带证明的
  guard-elimination explanation，以及输出 `CKR0007` 的 opt-in Native contract sanitizer。
- 用 C、WebAssembly、Native LLVM 共用的单一 verified KIR pipeline 取代旧 target-neutral
  MIR optimizer；semantic MIR 与稳定 `emit-mir` 仍负责 source order 和 first-error boundary。
- 新增 scalar/path、region alias 与 Memory SSA、interprocedural effect、loop、
  GVN/load-forwarding/dead-store、LICM 及可审计 backend fact。
- Native C ABI 保持 1；private LLVM bridge 与 runtime ABI 更新为 2，Native cache/codegen
  identity 使用 KIR v1。
- 新增 fixed-seed differential/mutation suite、pre-LLVM fact audit，以及相对固定 Clang 和
  精确 CalcKernel 0.10 的 performance gate。

## 0.10.0 - 2026-08-27

- 新增无参数 internal `main`、`ckc run` 与 Native executable output。
- 新增 signed/unsigned integer、`f64`、boolean、newline 的确定性 Native print builtin；
  library、C、WASM root 拒绝可达 print。
- 以固定 LLVM 22.1.8 structural codegen、ORC、archive writer 与进程内 LLD 取代产品
  Clang subprocess。
- `ckc build --kind` 扩展到 executable、dynamic、static、object；dynamic 仍为默认，
  `build-llvm` 成为 deprecated compatibility alias。
- Native object/static/dynamic export 统一为 generated-header Native C ABI，包含 target ABI
  classification 与 checked status thunk。
- Native 新增 checked overflow/slice bounds，并保持 C `CK_Status` meaning 与 first-error order。
- 新增 isolated run child、安全 persistent object cache、固定 runtime diagnostic/status、eager
  symbol resolution 与 JIT page-permission audit。
- 新增 checked/unchecked C-oracle performance gate，以及六 host functional、artifact、dependency、
  provenance 与 immutable release gate。
- 保留 `main` 与七个 print builtin 名称，将 Native target 限制为 host，退出 standalone LLVM
  export-shape promise，并保持 `emit-c` source-only。迁移见兼容性策略。

## 0.9.0 - 2026-08-26

- 新增 `while` 循环内的结构化控制语句 `break` 与 `continue`。
- 新增显式 `void` 过程、空 `return;` 以及过程调用语句。
- 新增非 owning 的 `slice<T>` 值、`slice(data, len)` 构造、索引、`.data` / `.len`
  访问，以及写作 `items[start..end]` 的半开 sub-slice。
- C backend 新增可选的 `--bounds checked` slice 边界检查；unchecked 仍是默认值，
  WASM 与 LLVM 会拒绝 checked bounds。
- 冻结原生 C、WebAssembly 与 LLVM 输出路径及其 V0.9 ABI。
- 按稳定的 compiler、contract、example、benchmark 与 test 职责整理仓库，同时保持
  compiler 的公共行为不变。
- 冻结 V0.9 兼容边界：`0.9.x` patch release 保持已接受源码、diagnostic ID、CLI
  行为、文本 MIR 和已记录 ABI contract 的向后兼容。后续 `0.10.0` 只可在提供迁移
  指南时引入已记录的破坏性变更；长期兼容承诺从未来的 `1.0.0` 开始。
- 为 macOS、Linux、Windows 的 arm64 与 x64 提供经过验收的原生 `ckc` 发布归档及
  SHA-256 checksum。
