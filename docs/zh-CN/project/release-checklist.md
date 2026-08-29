# CalcKernel 发布清单

[English](../../project/release-checklist.md)

对版本 `X.Y.Z`：

- [ ] `Cargo.toml`、`Cargo.lock`、README 与双语 changelog 都声明 `X.Y.Z`。
- [ ] Language、diagnostic、CLI、semantic MIR/KIR boundary、optimizer、ABI、compatibility、release docs 与实现一致。
- [ ] 英文/简体中文文档树镜像，所有 local link 可解析。
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] 使用 checksum-verified LLVM 22.1.8 release prefix 与 pinned Clang oracle 执行 `cargo test --all-features --locked`。
- [ ] `cargo build --release --features native-toolchain --locked`
- [ ] Generated/mutation suite 在 C、WebAssembly、Native 支持的 O0–O3 mode 下通过。
- [ ] 六 host pre-LLVM fact audit 通过并拒绝 mutation corpus。
- [ ] Contract sanitizer ownership test 在 ASan/UBSan 下通过。
- [ ] `./target/release/ckc --help`、`--version --verbose` 与 `licenses` 暴露完整 identity/notice evidence。
- [ ] `ckc run` 与 `ckc build --kind executable` 均在 external-tool `PATH` 为空时通过。
- [ ] 每个 host 的 generated artifact、release binary dependency 与 JIT memory audit 通过；hardened macOS 只使用允许的 allow-JIT entitlement。
- [ ] Controlled x86-64/AArch64 worker 上以 portable baseline CPU policy 执行的 Clang、精确 0.10、proof-loop checked/unchecked 与 optimizer-latency gate 均通过；native-CPU 测量只用于调查。
- [ ] 精确 release commit 的 main CI 为 green。
- [ ] publish=false 的六平台 preview 为 green。
- [ ] Annotated `vX.Y.Z` 指向精确 commit 且从未存在。
- [ ] Workflow 在 artifact build 前验证 tag 等于 `v` 加 `Cargo.toml` version。
- [ ] Release verification 自包含，不依赖可选的 TypeScript oracle checkout。
- [ ] Tag workflow 创建恰好六个 archive 与六个 SHA256 sidecar。
- [ ] 所有 checksum 验证成功；解压后的 native-enabled binary 通过 version、licenses、run、build、dependency 与 JIT smoke。
- [ ] 此 tag 尚无 Release；workflow 创建而不是覆盖 Release。
- [ ] GitHub Release 已 publish，非 draft/prerelease，关联 changelog 且恰有 12 个 asset。

禁止 force-push、移动 tag、覆盖 asset、跳过 target 或降低 gate。Tag 后缺陷必须使用
新 patch version。
