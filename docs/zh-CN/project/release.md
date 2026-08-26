# 原生 `ckc` 发布策略

[English](../../project/release.md)

CalcKernel 发布原生 `ckc` executable、source 与 documentation，不发布 JavaScript
wrapper 或 registry package。

`native ckc release` workflow 先运行 format、strict Clippy、test、release build、
CLI help 与 source/MIR smoke，再生成六个 archive：

- `ckc-darwin-arm64.tar.gz`
- `ckc-darwin-x64.tar.gz`
- `ckc-linux-arm64.tar.gz`
- `ckc-linux-x64.tar.gz`
- `ckc-win32-arm64.zip`
- `ckc-win32-x64.zip`

每个 archive 都有同名 `.sha256` sidecar。关闭 publish 的 manual run 是 release
preview。Tag run 只有在所有 platform 成功后，才从 tag 创建唯一 GitHub Release 并
上传 12 个 immutable asset。

Release tag 是 annotated `vMAJOR.MINOR.PATCH`，永不移动。Published Release 或
asset 不覆盖；若 `v0.9.0` 之后发现缺陷，发布 `v0.9.1` 等新 patch version。
[发布清单](release-checklist.md)是必须完成的 sign-off record。
