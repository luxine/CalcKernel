# 阶段 10 验收：CLI 与 contract sanitizer

## 环境

使用阶段 09 的 pinned LLVM/Clang 环境变量。

## 必须通过

1. `cargo test --locked --test cli kir_ -- --nocapture`
2. `cargo test --locked --test cli argument_ -- --nocapture`
3. `cargo test --all-features --locked --test native contract_sanitizer_ -- --nocapture`
4. `cargo test --all-features --locked --test native run_ -- --nocapture`
5. `cargo test --all-features --locked --test native cache_ -- --nocapture`
6. `cargo test --all-features --locked --test native runtime_ -- --nocapture`
7. `cargo test --locked --test backend header_ -- --nocapture`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --all-features --locked -- -D warnings`
10. `git diff --check`

## 结构断言

- inspection 输出重复运行 byte-identical，且不含绝对路径、地址、时间或 unordered map。
- normal O0–O3 Native IR 无 contract sanitizer guard/runtime symbol。
- 所有 violation 精确输出 `CKR0007: unsafe contract violation\n` 且 status=246。
- 极值/回绕 case 不 crash、不触发 host UB、不错误通过。
- sanitizer 只用于 run/executable；library/emit 命令在写文件前拒绝。
- header comments 可机械映射 flattened slice fields，C ABI declaration shape 不变。

## 完成证据

执行时追加 SHA、runtime/bridge identity、每个 O-level sanitizer matrix 与 exact-byte 摘要。

## 通过证据（2026-08-29）

- 实现提交：`eab9a30600c029e0664cc2dd4982ce82a9920c29`。
- identity：LLVM `22.1.8`；Native ABI `1`；Runtime ABI `2`；private Bridge ABI `2`；
  本地刷新后的 pinned manifest SHA-256 为
  `5956812de80c9ecc0d7765a8635d89e7f78062de7f178cdc58d7c3f78509111c`。
- 以上 10 条必须命令全部返回成功；CLI KIR `2/2`、argument `1/1`、Native
  sanitizer `6/6`、run `5/5`、cache `6/6`、runtime `6/6`、header `1/1`，fmt、
  all-feature clippy `-D warnings` 与 `git diff --check` 均通过。
- O0/O1/O2/O3 负向 unsafe-call matrix 均精确得到 stderr
  `CKR0007: unsafe contract violation\n`、空 stdout、退出状态 `246`；同一四级 matrix
  的合法多谓词输入均退出 `0`，递归边界在四级优化下均重新检查。
- 极值覆盖无界大系数 affine、`u64::MAX`、`multiple_of`、pairwise `noalias`、32-byte
  alignment、相同地址零长度 slice、非零重叠和 `usize::MAX` 邻近的地址端点回绕；无
  crash/host UB，违反统一返回 status `7`。normal O0–O3 LLVM IR 均不含
  `contract.sanitize` 或 `__ck_contract_`。
- inspection 重复运行 stdout/stderr byte-identical，不含源文件绝对路径、地址或时间；
  header 注释将 slice 规范化为 `name_data[0..name_len]`，过滤注释后的 ABI declaration
  bytes/shape 不变；normal/sanitizer cache 各自产生独立对象条目。
- affine 采用按契约常量位宽计算的 LLVM arbitrary-width integer（最低 128 bit）作为
  exact overflow-safe evaluator；地址区间固定用 192 bit 数学运算并显式拒绝超出 64-bit
  目标地址宽度的端点。sanitizer 模式不发出 contract-derived LLVM 属性、assume、alias
  metadata 或 wrap flags，避免检查被未验证先验消除。
