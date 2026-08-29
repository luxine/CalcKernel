# 原生 `ckc` 0.11 发布策略

[English](../../project/release.md)

CalcKernel 发布原生 `ckc` executable、source 与 documentation，不发布 JavaScript
wrapper 或 registry package。

`native ckc release` workflow 在本仓库内自包含，不依赖外部 source checkout。
所有 action 均锁定到完整 commit。Workflow 只获取 `native/llvm/manifest.toml` 指定的
LLVM 22.1.8 source archive，验证 SHA-256，并恢复或构建以 manifest 寻址的 host
cache。验证任务使用独立的 pinned Clang oracle prefix；发行构建使用排除 Clang 的
target-minimal `release` profile，并始终执行 `cargo build --release --features
native-toolchain --locked`。

打包前，每个 host 都记录 `ckc --version --verbose` 与 `ckc licenses`，实际执行
`ckc run` 和 `ckc build --kind executable` 生成的 standalone executable，并运行
generated-artifact、compiler dependency 与 JIT memory-permission audit。Linux 与
Windows release 不得保留 dynamic non-system C++ runtime；Darwin 依赖只能解析到
Apple system library。macOS 还要在 hardened runtime 下验证唯一的
`com.apple.security.cs.allow-jit` entitlement。Audit 显式提取 XML 并比较 canonical binary
plist，不解析随系统版本变化的 `codesign`/`plutil` 人类可读输出。JIT audit 必须验证与 runtime capability
一致的 Darwin W^X 路径：per-thread `MAP_JIT`，或在不支持 per-thread 时用页级
RW/NX-to-RX/R-NX finalization；永不接受 RWX fallback。Tag run 必须在任何 artifact job
启动前验证 tag 等于 `v` 加 `Cargo.toml` 中的版本。随后生成六个 archive：

- `ckc-darwin-arm64.tar.gz`
- `ckc-darwin-x64.tar.gz`
- `ckc-linux-arm64.tar.gz`
- `ckc-linux-x64.tar.gz`
- `ckc-win32-arm64.zip`
- `ckc-win32-x64.zip`

每个 archive 仅含一个完整、native-enabled 的 `ckc`，并有同名 `.sha256` sidecar，
记录路径为 archive basename。关闭 publish 的 manual run 是 release preview。Tag
run 验证完整的六个 archive 与六个 checksum；若 Release 已存在则失败，否则以
`CHANGELOG.md` 创建唯一 GitHub Release 并上传 12 个 immutable asset。只有最终
publish job 具有 repository write permission。

Release tag 是 annotated `vMAJOR.MINOR.PATCH`，永不移动。Published Release 或
asset 不覆盖；若 `v0.11.0` 之后发现缺陷，发布 `v0.11.1` 等新 patch version。0.11.0
发布由六个 archive 和对应六个 checksum sidecar 组成，必须 all-or-nothing 发布。
[发布清单](release-checklist.md)是必须完成的 sign-off record。
