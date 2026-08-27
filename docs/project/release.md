# Native `ckc` Release Policy

[简体中文](../zh-CN/project/release.md)

CalcKernel releases the native `ckc` executable, source, and documentation. It
does not publish a JavaScript wrapper or registry package.

The `native ckc release` workflow is self-contained in this repository; it does
not check out the former TypeScript implementation. Every action is pinned to a
full commit. The workflow acquires only the LLVM 22.1.8 source archive named in
`native/llvm/manifest.toml`, verifies its SHA-256, and restores or builds a
manifest-addressed host cache. Verification uses a separate pinned Clang oracle
prefix. Distribution builds use the target-minimal `release` profile, which
excludes Clang, and always run `cargo build --release --features
native-toolchain --locked`.

Before packaging, each host records `ckc --version --verbose` and `ckc
licenses`, exercises both `ckc run` and a standalone executable produced by
`ckc build --kind executable`, and runs the generated-artifact, compiler
dependency, and JIT memory-permission audits. Linux and Windows releases may
not retain a dynamic non-system C++ runtime; Darwin dependencies must resolve
only to Apple system libraries. macOS additionally tests the hardened runtime
with only the `com.apple.security.cs.allow-jit` entitlement. On a tag run,
verification requires the tag to equal `v` plus the version in `Cargo.toml`
before any artifact job starts. The workflow then builds these six archives:

- `ckc-darwin-arm64.tar.gz`
- `ckc-darwin-x64.tar.gz`
- `ckc-linux-arm64.tar.gz`
- `ckc-linux-x64.tar.gz`
- `ckc-win32-arm64.zip`
- `ckc-win32-x64.zip`

Each archive contains one complete native-enabled `ckc` and has a same-name
`.sha256` sidecar whose recorded path is the archive basename. A manual run with
publishing disabled is the release preview. A tag run verifies the complete set
of six archives and six checksums, rejects an already existing Release, and
creates one GitHub Release from `CHANGELOG.md` with all twelve immutable assets
only after every platform succeeds. Repository write permission exists only in
that final publish job.

Release tags are annotated `vMAJOR.MINOR.PATCH` tags and are never moved. A
published Release or asset is never overwritten. If a defect is discovered
after `v0.9.0`, fix it in a new patch release such as `v0.9.1`.

The [release checklist](release-checklist.md) is the required sign-off record.
