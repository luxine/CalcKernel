# CalcKernel Release Checklist

[简体中文](../zh-CN/project/release-checklist.md)

For a version `X.Y.Z`:

- [ ] `Cargo.toml`, `Cargo.lock`, READMEs, and both changelogs name `X.Y.Z`.
- [ ] Language, diagnostics, CLI, semantic MIR/KIR boundary, optimizer, ABI, compatibility, and release docs match implementation.
- [ ] English and Simplified Chinese documentation trees mirror and local links resolve.
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --all-features --locked` against the checksum-verified LLVM 22.1.8 release prefix and pinned Clang oracle.
- [ ] `cargo build --release --features native-toolchain --locked`
- [ ] Generated and mutation suites pass O0–O3 for C, WebAssembly, and Native supported modes.
- [ ] Pre-LLVM fact audit passes on all six hosts and rejects the mutation corpus.
- [ ] Contract sanitizer ownership tests pass under ASan/UBSan.
- [ ] `./target/release/ckc --help`, `--version --verbose`, and `licenses` expose complete identity and notice evidence.
- [ ] `ckc run` and `ckc build --kind executable` both pass with no external-tool `PATH`.
- [ ] Generated artifact, release binary dependency, and JIT memory audits pass on every host; hardened macOS uses only the approved allow-JIT entitlement and selects a capability-consistent per-thread MAP_JIT or page-level W^X path, never RWX.
- [ ] The actual packaged Darwin compiler is explicitly ad-hoc signed with hardened runtime and the sole allow-JIT entitlement before strict signature verification.
- [ ] Strict schema 7 Clang, exact-0.11/0.10 replay, C/Rust SIMD, domain-fact,
  proof-loop, optimizer-latency, object-size, and source-to-object gates pass on
  controlled x86-64 and AArch64 workers under portable baseline CPU policy;
  native-CPU measurements remain investigative.
- [ ] Main-branch CI is green at the exact release commit.
- [ ] The manual six-platform release preview is green with publishing disabled.
- [ ] The annotated tag `vX.Y.Z` points to that exact commit and has never existed before.
- [ ] The workflow verifies that the tag equals `v` plus the `Cargo.toml` version before artifact builds.
- [ ] Release verification is self-contained and does not require the optional TypeScript oracle checkout.
- [ ] The tag-triggered workflow creates exactly six archives and six SHA256 sidecars.
- [ ] Every archive checksum verifies and each extracted native-enabled binary passes version, licenses, run, build, dependency, and JIT smoke checks.
- [ ] No Release already exists for the tag; the workflow creates, rather than overwrites, it.
- [ ] The GitHub Release is published, non-draft, non-prerelease, links the changelog, and has exactly twelve assets.

Do not force-push, move a tag, overwrite an asset, skip a target, or weaken a
gate. A post-tag defect requires a new patch version.
