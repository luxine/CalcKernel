# Native `ckc` 0.11 Release Policy

[简体中文](../zh-CN/project/release.md)

CalcKernel releases the native `ckc` executable, source, and documentation. It
does not publish a JavaScript wrapper or registry package.

Repository text checks out with LF endings on every host. Vendored provenance
files retain their original bytes without Git newline conversion; hash checks
compare those exact bytes and never normalize the input to accept a mismatch.

The `native ckc release` workflow is self-contained in this repository and does
not depend on an external source checkout. Every action is pinned to a full
commit. The workflow acquires only the LLVM 22.1.8 source archive named in
`native/llvm/manifest.toml`, verifies its SHA-256, and restores or builds a
manifest-addressed host cache. Its identity also covers every compiled native
runtime and platform-link input, and a newly built prefix is saved immediately
after its manifest and object hashes pass independent validation. The release
prefix is saved before the separate Clang oracle build begins, so an oracle
failure cannot discard the already verified compiler toolchain. Verification
uses a separate pinned Clang oracle prefix.
Distribution builds use the target-minimal `release` profile, which
excludes Clang, and always run `cargo build --release --features
native-toolchain --locked`.

Both macOS CI hosts and release artifact jobs explicitly ad-hoc sign the actual
compiler with hardened runtime and the repository's sole allow-JIT entitlement
before strict signature audits. The signed compiler is the one packaged; testing
only a signed temporary copy is insufficient. This is not Developer ID signing
or notarization, and no signing credentials are required.

Before packaging, each host records `ckc --version --verbose` and `ckc licenses`,
exercises both `ckc run` and a standalone executable produced by
`ckc build --kind executable`, and runs the generated-artifact, compiler
dependency, and JIT memory-permission audits. Linux and Windows releases may
not retain a dynamic non-system C++ runtime; Darwin dependencies must resolve
only to Apple system libraries. macOS additionally tests the hardened runtime
with only the `com.apple.security.cs.allow-jit` entitlement. Its JIT audit
extracts explicit XML and compares canonical binary plists, rather than parsing
version-dependent human-readable `codesign` or `plutil` output. The audit
requires the runtime-capability-consistent Darwin W^X path: either per-thread
`MAP_JIT`, or page-level RW/NX-to-RX/R-NX finalization when per-thread support is
unavailable; an RWX fallback is never accepted. Darwin AOT and ORC objects use
PIC with the Small code model; unoptimized internal calls are checked for
absolute executable-text relocations. Standalone executables test dyld's normal
C-ABI `LC_MAIN` invocation and exact exit/stdio behavior. On a tag run,
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
after `v0.11.0`, fix it in a new patch release such as `v0.11.1`. The 0.11.0
release consists of six archives and their six checksum sidecars; publication
is all-or-nothing.

The [release checklist](release-checklist.md) is the required sign-off record.
