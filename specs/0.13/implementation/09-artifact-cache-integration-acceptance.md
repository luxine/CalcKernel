# 阶段 09 验收：named-object artifact、cache 4 与 CLI 集成

## 必须通过

1. `cargo test --all-features --locked --test native multiversion_artifact_ -- --nocapture`
2. `cargo test --all-features --locked --test native cache_ -- --nocapture`
3. `cargo test --all-features --locked --test native libraries_ -- --nocapture`
4. `cargo test --all-features --locked --test cli build_transaction_ -- --nocapture`
5. `cargo test --locked --test contracts native_cache_ -- --nocapture`
6. `cargo test --all-features --locked`
7. `cargo build --release --features native-toolchain --locked`
8. `scripts/audit-native-artifact.sh target/native-acceptance/v0.13-stage-09`
9. `scripts/audit-ckc-release.sh target/release/ckc`
10. `cargo fmt --check`
11. `cargo clippy --all-targets --all-features --locked -- -D warnings`
12. `git diff --check`

所有 filter 非零；artifact matrix 必须使用 real linker/archive，不得只断言内存 object list。

## 结构断言

- multiversion 只产生 executable/dynamic/static；object 组合在输出前拒绝，single-version use object 正常。
- cache 是 CKCOBJ03/key+manifest 4；complete dispatcher+variant manifest 验证，任一 missing/extra/
  reorder/redirect/digest/schema mismatch 导致全 bundle miss/reject，generate 永不 cache。
- final artifact 无 profile writer/runtime/path/counter/flush/LLVM/compiler/new shared dependency；public
  symbols/header/ABI stable，private symbols hidden/namespace/feature-contained。
- output set 全部原子 rollback；相同 canonical 输入在 cwd/order/cache hit/miss 间 byte-reproducible。

## 完成证据

记录实现 SHA、cache/artifact manifests SHA、real artifact matrix、transaction mutation、release/native
audit 与命令结果。阶段 09 尚未把版本声明升级到 0.13.0。
