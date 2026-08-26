# Native `ckc` Release Policy

[简体中文](../zh-CN/project/release.md)

CalcKernel releases the native `ckc` executable, source, and documentation. It
does not publish a JavaScript wrapper or registry package.

The `native ckc release` workflow first runs format, strict Clippy, tests,
release build, CLI help, and source/MIR smoke checks. It then builds these six
archives:

- `ckc-darwin-arm64.tar.gz`
- `ckc-darwin-x64.tar.gz`
- `ckc-linux-arm64.tar.gz`
- `ckc-linux-x64.tar.gz`
- `ckc-win32-arm64.zip`
- `ckc-win32-x64.zip`

Each archive has a same-name `.sha256` sidecar. A manual run with publishing
disabled is the release preview. A tag run creates one GitHub Release from the
tag and uploads the twelve immutable assets only after every platform succeeds.

Release tags are annotated `vMAJOR.MINOR.PATCH` tags and are never moved. A
published Release or asset is never overwritten. If a defect is discovered
after `v0.9.0`, fix it in a new patch release such as `v0.9.1`.

The [release checklist](release-checklist.md) is the required sign-off record.
