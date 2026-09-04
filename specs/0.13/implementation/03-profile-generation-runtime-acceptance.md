# 阶段 03 验收：generation pipeline、collector 与 flush

## 必须通过

在固定 LLVM 22.1.8 prefix 下：

1. `cargo test --all-features --locked --test native profile_generation_ -- --nocapture`
2. `cargo test --all-features --locked --test cli pgo_build_ -- --nocapture`
3. `cargo test --all-features --locked --test profile generation_ -- --nocapture`
4. `cargo test --locked --test contracts native_toolchain_ -- --nocapture`
5. `cargo build --release --features native-toolchain --locked`
6. `scripts/test-sanitized-ownership.sh`
7. `scripts/audit-native-artifact.sh target/native-acceptance/v0.13-stage-03`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --all-features --locked -- -D warnings`
10. `git diff --check`

所有 feature/filter 必须运行非零测试；六主机差异留给阶段 11，但本机支持的 real runtime path
不能只用 mock 代替。

## 结构断言

- ordinary/use artifact 不含 counter、path、flush symbol 或 profile runtime；generation cache miss
  是强制行为。
- compiler-private initialization guard 必须保留 `NoInline`；generation object 的 hot instrumented
  function/loop site 只能调用紧凑 guard，不得重复展开完整 initialization 参数准备。
- executable 只在 normal zero-result automatic workflow 接受 shard；library flush 是完整 64-hex、
  exactly-one publisher、concurrent/repeat sticky，unload path 无 I/O。
- directory every component no-follow/identity anchored，replacement/symlink/reparse/overwrite 被拒绝；
  completed shard 经自身 parser/digest 校验后才 publication。
- CLI 任何 child/profile/final-build 失败都保留 prior outputs，且无 partial final profile/artifact。

## 完成证据

记录实现 SHA、Rust/LLVM/Clang/host identity、real executable/library shard SHA、并发 flush 结果、
artifact symbol/import audit 与 test count。阶段 03 不得声称 profile 已影响优化。
