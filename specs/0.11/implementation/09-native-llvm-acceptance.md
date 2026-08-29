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
