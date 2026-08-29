# 阶段 09 验收：Native LLVM 与事实审计

## 环境

```sh
export CKC_LLVM_PREFIX=/Users/lynn/code/Rust_CalcKernel/.worktrees/native-toolchain-0.10/build/llvm/prefix-aarch64-apple-darwin11-release
export CKC_CLANG_ORACLE=/Users/lynn/code/Rust_CalcKernel/.worktrees/native-toolchain-0.10/build/llvm/prefix-aarch64-apple-darwin11-oracle/bin/clang
```

## 必须通过

1. `cargo test --all-features --locked --test native fact_audit_ -- --nocapture`
2. `cargo test --all-features --locked --test native llvm_ -- --nocapture`
3. `cargo test --all-features --locked --test native abi_ -- --nocapture`
4. `cargo test --all-features --locked --test native differential_ -- --nocapture`
5. `cargo test --all-features --locked --test native object_ -- --nocapture`
6. `cargo test --all-features --locked --test native libraries_ -- --nocapture`
7. `cargo test --all-features --locked --test native executable_ -- --nocapture`
8. `cargo test --all-features --locked --test native jit_ -- --nocapture`
9. `cargo fmt --check`
10. `cargo clippy --all-targets --all-features --locked -- -D warnings`
11. `git diff --check`

## 结构断言

- untracked CK-owned strengthening mutations 全部被 pre-optimization audit 拒绝。
- LLVM 优化后自行推导属性不会被错误归为 CK-owned。
- third-root/capture/return 反例无 parameter noalias；合法 pair 仅有 scoped metadata。
- O0–O3 Native checked/unchecked differential、C ABI shape 和 artifact transaction 全绿。
- private bridge ABI=2；Native public ABI 仍为 1。

## 完成证据

执行时追加 SHA、LLVM identity、host triple、audit property/mutation 数与 differential matrix。

## 执行记录（2026-08-29）

- 实现提交：`ac69fb7e119f1033b56b27eca54c36869796b74e`。
- Native KIR lowering 直接消费 evidence-verified KIR；scalar、control、slice、checked
  ordered guards、runtime calls、C ABI thunks 与 executable entry wrapper 均不经过 MIR optimizer，
  也不重新推断已经从 KIR 删除的检查。
- private bridge ABI 为 `2`；Native ABI 与 Runtime ABI 继续为 `1`。桥接 LLVM identity 为
  `22.1.8`，manifest SHA-256 为
  `2e00d1c91a268879cd262a15d7120edc5d10a3af8086742ca574ff5f3e8bdbc8`；host
  target 为 `aarch64-apple-darwin`，code generator 为 `AArch64`，ORC layer 为
  `JITLink`。
- CK-owned whitelist 覆盖 range/`llvm.assume`、alignment、`nuw`/`nsw`、parameter
  readonly/writeonly、function memory effects、parameter noalias 与 access-scoped
  `alias.scope`/`noalias` metadata；每个记录均携带 `FactId` 或 `ProofId`。完整双根正例的
  audit property 数为 `10`。阶段 11 复诊发现原测试侧信道不足后，提交 `83ee0a1` 改为
  从 pre-optimization LLVM IR 实际枚举各类 strengthening 并与 Fact/Proof map 对账；真实
  未登记的 `noalias` attribute 与不会设置测试标志的 `nuw` flag 两类 mutation 均被拒绝。
- noalias 负例覆盖第三根、checked 非 void 隐藏结果指针、slice 返回与未内联调用边界；
  均无 parameter noalias。部分 pair 仍只在对应根的真实 load/store 上生成 scoped
  metadata。
- differential matrix 为 overflow/bounds 同步的 unchecked 与 checked 两组乘 O0、O1、
  O2、O3，共 `8` 格；每格将 Native KIR 动态库与 pinned Clang C oracle 比较 scalar、
  control、void/pointer mutation、struct、slice、checked 首错和除零行为，全部一致。
- 回归计数：fact audit `7/7`、LLVM filter `17/17`、ABI filter `5/5`、differential
  `1/1`（内部 8 格）、object `5/5`、libraries `3/3`、executable `3/3`、JIT
  `8/8`。
- Rust 工具链：`rustc 1.90.0 (1159e78c4 2025-09-14)`、Rust LLVM `20.1.8`、
  `cargo 1.90.0 (840b83a10 2025-07-30)`。
- 本文件“必须通过”第 1–11 项全部以 exit status `0` 通过。
