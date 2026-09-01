# 阶段 08 验收：CPU detector、dispatcher 与 thunk

## 必须通过

在固定 Native toolchain 下：

1. `cargo test --all-features --locked --test native multiversion_dispatch_ -- --nocapture`
2. `cargo test --all-features --locked --test native detector_ -- --nocapture`
3. `cargo test --all-features --locked --test native abi_ -- --nocapture`
4. `cargo test --all-features --locked --test native differential_ -- --nocapture`
5. `cargo test --all-features --locked --test native ownership_ -- --nocapture`
6. `cargo build --release --features native-toolchain --locked`
7. `scripts/audit-native-artifact.sh target/native-acceptance/v0.13-stage-08`
8. `scripts/test-sanitized-ownership.sh`
9. `cargo fmt --check`
10. `cargo clippy --all-targets --all-features --locked -- -D warnings`
11. `git diff --check`

所有 filter 非零；real detector 在本机执行，其他 host table 用 fixture mutation，阶段 11 再由六主机
验证真实 platform path。

## 结构断言

- detector/query failure/unknown/contradictory/unsupported OS 均 baseline；x86 检查硬件与 OS state，
  Linux AArch64 只用 initial auxv state。
- public address 恒为 baseline-safe thunk；variant/runtime symbol hidden，ABI/header/export bytes 与
  单版本一致；static private symbol 含 target-set digest namespace。
- capability cache 恰一次，后续 steady call 是 atomic load + indirect tail call；concurrent first calls
  只发布 compatible verified pointer。
- baseline/thunk/detector 无 optional instruction，variant 不越声明 feature，无 cross-module leakage。

## 完成证据

记录实现 SHA、dispatch runtime identity、capability manifest、并发/real-hardware selection、symbol/
disassembly/differential audit。阶段 08 通过不代签 final bundle/cache transaction。
