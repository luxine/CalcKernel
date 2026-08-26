# 原生 `ckc` 发布策略

[English](../../project/release.md)

CalcKernel 发布原生 `ckc` executable、source 与 documentation，不发布 JavaScript
wrapper 或 registry package。

`native ckc release` workflow 在本仓库内自包含，不 checkout 或构建可选的
TypeScript oracle。它先运行 format、strict Clippy、locked test、release build、CLI
help 与 source/MIR smoke。Tag run 还必须在任何 artifact job 启动前，验证 tag 等于
`v` 加 `Cargo.toml` 中的版本。随后生成六个 archive：

- `ckc-darwin-arm64.tar.gz`
- `ckc-darwin-x64.tar.gz`
- `ckc-linux-arm64.tar.gz`
- `ckc-linux-x64.tar.gz`
- `ckc-win32-arm64.zip`
- `ckc-win32-x64.zip`

每个 archive 都有同名 `.sha256` sidecar，记录的路径为 archive basename。关闭
publish 的 manual run 是 release preview。Tag run 在所有 platform 成功后验证六个
checksum；若 Release 已存在则失败，否则以 `CHANGELOG.md` 创建唯一 GitHub Release
并上传 12 个 immutable asset。只有最终 publish job 具有 repository write permission。

Release tag 是 annotated `vMAJOR.MINOR.PATCH`，永不移动。Published Release 或
asset 不覆盖；若 `v0.9.0` 之后发现缺陷，发布 `v0.9.1` 等新 patch version。
[发布清单](release-checklist.md)是必须完成的 sign-off record。
