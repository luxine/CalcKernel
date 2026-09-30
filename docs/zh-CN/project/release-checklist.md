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
- [ ] 对有 WebAssembly codegen 变化的版本，严格 `baseline` 与 `simd128` 正确性/性能证据须标明精确候选源码与编译器二进制。对照已验收开发候选报告，核验正式构建的 module hash、所需 feature 声明与正确性输出；若生成 module 不同，则重测并单独标明正式构建数据。冷调用和模块体积单独于热调用比率报告，并明确标注开发候选实测结果。
- [ ] 六 host pre-LLVM fact audit 通过并拒绝 mutation corpus。
- [ ] Contract sanitizer ownership test 在 ASan/UBSan 下通过。
- [ ] `./target/release/ckc --help`、`--version --verbose` 与 `licenses` 暴露完整 identity/notice evidence。
- [ ] `ckc run` 与 `ckc build --kind executable` 均在 external-tool `PATH` 为空时通过。
- [ ] 每个 host 的 generated artifact、release binary dependency 与 JIT memory audit 通过；hardened macOS 只使用允许的 allow-JIT entitlement，并按 capability 选择 per-thread MAP_JIT 或页级 W^X，绝不接受 RWX。
- [ ] 实际打包的 Darwin compiler 在严格签名校验前已显式 ad-hoc 签名，启用 hardened runtime，且只含唯一 allow-JIT entitlement。
- [ ] Controlled x86-64/AArch64 worker 上 strict schema 8 exact-0.12 replay、CK
  PGO/multiversion/combined、selected-direct dispatch、等价 Clang/Rust PGO、C/Rust SIMD、
  domain-fact、generation overhead、optimizer latency、artifact/archive size 与
  source-to-object gate 均在 portable baseline CPU policy 与 required enhanced tier 下通过。
- [ ] Profile、target-set、variant-object、capability、hardware、recipe、sample、source、
  oracle 与 candidate identity/digest 完整且精确。
- [ ] exact candidate-SHA 的十个 required CI job 全绿，无 skipped/continue-on-error gate；
  两个 performance worker 都具备 required enhanced tier。
- [ ] publish=false 的六平台 preview 为 green。
- [ ] Annotated `vX.Y.Z` 指向精确 commit 且从未存在。
- [ ] Workflow 在 artifact build 前验证 tag 等于 `v` 加 `Cargo.toml` version。
- [ ] Quality 在执行 live differential gate 前，通过
  `tests/oracles/typescript/SOURCE_MANIFEST.sha256` 校验并构建仓库内的 TypeScript oracle。
- [ ] Release verification 自包含，不依赖仅供测试的 TypeScript oracle。
- [ ] Tag workflow 创建恰好六个 archive 与六个 SHA256 sidecar。
- [ ] 所有 checksum 验证成功；解压后的 native-enabled binary 通过 version、licenses、run、build、dependency 与 JIT smoke。
- [ ] 此 tag 尚无 Release；workflow 创建而不是覆盖 Release。
- [ ] GitHub Release 已 publish，非 draft/prerelease，关联 changelog 且恰有 12 个 asset。

禁止 force-push、移动 tag、覆盖 asset、跳过 target 或降低 gate。Tag 后缺陷必须使用
新 patch version。
