# 阶段 05 验收：O2 CK late machine layout 与 bridge ABI 4

## 必须通过

在固定 LLVM 22.1.8 prefix 下：

1. `cargo test --all-features --locked --test native pgo_layout_ -- --nocapture`
2. `cargo test --all-features --locked --test native bridge_abi_ -- --nocapture`
3. `cargo test --all-features --locked --test native fact_audit_ -- --nocapture`
4. `cargo test --all-features --locked --test cli pgo_o2_ -- --nocapture`
5. `cargo test --locked --test contracts native_toolchain_ -- --nocapture`
6. `cargo build --release --features native-toolchain --locked`
7. `scripts/audit-native-artifact.sh target/native-acceptance/v0.13-stage-05`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --all-features --locked -- -D warnings`
10. `git diff --check`

所有 filter 非零；layout 测试同时覆盖 accepted 和 conservative fallback，不能只测 mock plan。

## 结构断言

- O2 profile-on/off 在 late boundary 前 snapshot byte-identical，LLVM IR/Machine state 无 profile
  metadata/attribute，所有 ordinary structural transform 已结束。
- verifier 只接受 closed structural delta；任何非 terminator 指令变化、duplicate/delete/CFG/call-target/
  reschedule 或未列 target repair 都被拒绝并保留 ordinary order。
- AArch64 accepted layout 执行 required branch relaxation；CFI/unwind/LOH/security/bundle 审计不因
  layout 被弱化。
- bridge ABI 是 4，Rust/C schema/ownership/error 路径一致；malformed plan 不产生 object/cache entry。

## 完成证据

记录实现 SHA、bridge/LLVM/target identity、pre/post digest、allowlist/fallback 矩阵与 object audit。
阶段 05 通过不代表 O3 PGO 或 multiversion 已实现。
