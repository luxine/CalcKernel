# CalcKernel 发布清单

[English](../../project/release-checklist.md)

对版本 `X.Y.Z`：

- [ ] `Cargo.toml`、`Cargo.lock`、README 与双语 changelog 都声明 `X.Y.Z`。
- [ ] Language、diagnostic、CLI、MIR、ABI、compatibility、release docs 与实现一致。
- [ ] 英文/简体中文文档树镜像，所有 local link 可解析。
- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `cargo test --locked`
- [ ] `cargo build --release --locked`
- [ ] `./target/release/ckc --help` 与代表性 check/O0–O3 backend smoke。
- [ ] 精确 release commit 的 main CI 为 green。
- [ ] publish=false 的六平台 preview 为 green。
- [ ] Annotated `vX.Y.Z` 指向精确 commit 且从未存在。
- [ ] Tag workflow 创建恰好六个 archive 与六个 SHA256 sidecar。
- [ ] 所有 checksum 验证成功，解压 binary 可打印 CLI help。
- [ ] GitHub Release 已 publish，非 draft/prerelease，并关联 changelog。

禁止 force-push、移动 tag、覆盖 asset、跳过 target 或降低 gate。Tag 后缺陷必须使用
新 patch version。
