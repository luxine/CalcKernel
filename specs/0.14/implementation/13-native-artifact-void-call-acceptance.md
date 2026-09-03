# 阶段 13 验收：Host Artifact 与 Void Call

## 本地必须通过

- [ ] `cargo test --all-features --locked --test cli multiversion_build_should_commit_the_verified_stage09_artifact_bundle -- --nocapture`
- [ ] `cargo test --all-features --locked --test native named_void_call_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test native profile_generation_ -- --nocapture`
- [ ] `cargo test --locked --test contracts ci_v014_native_fulfillment_ -- --nocapture`

## 结构断言

- [ ] dynamic primary/header/import-library 的路径全部来自
  `NativeArtifactPaths`，六个平台不含硬编码 `.dylib/.so/.dll` 测试假设。
- [ ] 非 void call 保持请求名称；void call 即使收到非空名称也不设置 SSA
  name，且 LLVM module verify 成功。
- [ ] Bridge ABI 4、Rust FFI、handle ownership 与错误状态不变。
- [ ] 六 host CI 在同一 required native job 内真实运行 executable、dynamic、
  profile-generation 与 void-call selector，无 skip/continue-on-error。

## 完成证据

命令、测试数、LLVM 版本和产物路径清单写入
`target/acceptance/v0.14/stage-13/`。
