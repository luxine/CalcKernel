# 原生 `ckc` 0.15 发布策略

[English](../../project/release.md)

CalcKernel 发布原生 `ckc` executable、source 与 documentation，不发布 JavaScript
wrapper 或 registry package。

不可移动的公开 `v0.15.0` tag 记录了 compiler source snapshot，但没有生成官方 GitHub
Release 或 compiler archive。`0.15.1` 是 0.15 系列首个正式发布版本。`0.15.2` 定位为
WebAssembly 性能与正确性更新，并保持相同的公共兼容边界。其 release workflow 会校验 annotated
tag，同时保留精确 tag source 和全部现有发布门禁。
官方 compiler archive 仅从 tag 与 `Cargo.toml` 匹配的公开 compiler commit 构建；workflow
直接使用该 checkout 的源码，不会从其他仓库 checkout source。Schema-7/8 performance gate
与六平台 Native 仍是必要发布门禁；离线 Auto-Tuning 延期。Node/V8 WebAssembly map 测量是
特定工作负载的本地观察结果，不是可移植的 release threshold，也不能概括为普遍的性能提升。

严格 11-kernel WebAssembly parity 套件在 Apple M5 Max、Node.js 24.14.0/V8 上记录了两次
七轮开发候选运行。候选源码提交为 `08292f18b6374f9636bae5ab07fbfcb8e9e30091`，`ckc` 二进制
SHA-256 为 `5db7c4db4146dda00020941f1047a1ade794741374e9446af4b2669de8979e3b`；报告中的 compiler
版本为 `ckc 0.15.1`，报告 SHA-256 分别为
`ed56abd762d3d02404ae92b09ce213f8e8ef95f378020dea0f4b7355f302351e` 和
`4384dc2262a4a967a40d1b6f04096235def2b51408c12ae9c363c7946243378b`。Baseline 几何平均为
1.117× 和 1.118×，最低项为 0.920× 和 0.914×；
SIMD128 几何平均为 1.016× 和 1.008×，最低项为 0.923× 和 0.920×。这些精确候选记录是开发
阶段证据，不是 v0.15.2 正式 archive 的实测结果。它们测量热调用，不含进程启动、编译、fixture
I/O、module 实例化或内存增长。候选模块体积为 baseline 28,325 字节、SIMD128 34,941 字节；
部分 kernel 的首次调用延迟仍明显较高且因负载而异。发布性能材料必须单独说明这些成本；只有绑定
到正式构建及其生成 module 的验证结果，才能描述正式 archive 的性能。

Tag event filter 有意保持宽泛，使不合法的发布候选 tag 能在 workflow gate 中明确失败。正式发布
必须使用匹配 `vMAJOR.MINOR.PATCH` 稳定版本格式且带注释的 tag，并与 checkout 中的 Cargo 版本
一致；`-dev` tag 和轻量 tag 会在构建 artifact 前被拒绝。手动发布也执行相同校验。

新版本发布前，必须通过 schema-7/8 x86-64/AArch64 performance gate、全部六个平台 Native
以及精确 candidate-SHA 十作业 CI。PR 合并后，还需在精确 main SHA 上重新完成十作业 CI 与
关闭发布的六平台 release preview，才能创建 annotated tag。代码或 contract 变化后必须在新
SHA 上重跑受影响 gate，不沿用旧证据。

仓库文本在所有 host 上都以 LF 换行 checkout。Vendor provenance 文件保留上游原始字节，
不经 Git 换行转换；hash 校验始终比较精确字节，不通过规范化输入来接受不匹配。

`native ckc release` workflow 在本仓库内自包含，不依赖外部 source checkout。
所有 action 均锁定到完整 commit。Workflow 只获取 `native/llvm/manifest.toml` 指定的
LLVM 22.1.8 source archive，验证 SHA-256，并恢复或构建以 manifest 寻址的 host
cache。该 identity 还覆盖所有参与编译的 native runtime 与 platform-link input；新 prefix
在独立 manifest/object hash 验证后立即保存；release prefix 保存先于 Clang oracle build，
后者失败不能丢弃已验证的 compiler toolchain。验证任务使用独立的 pinned Clang oracle
prefix；发行构建使用排除 Clang 的
target-minimal `release` profile，并始终执行 `cargo build --release --features
native-toolchain --locked`。
Distributed compiler archive 内嵌构建全部受支持 artifact 所需的 private generation/dispatch
runtime object 与 notice；生成的 user artifact 仍保持 self-contained，不增加外部 runtime dependency。

Windows ARM64 profile runtime 使用 `/forceInterlockedFunctions-` 编译，使 freestanding
object 保留 baseline inline atomic，而不依赖 MSVC 默认生成的 CRT outline helper。
Bootstrap 在接受 prefix 前检查已编译 object 的 undefined symbol，拒绝任何
`_Interlocked*` import。此规则不提高受支持的 CPU baseline。

macOS CI host 与 release artifact job 都必须在严格签名审计前，给实际 compiler 显式添加
ad-hoc hardened-runtime 签名，且只使用仓库唯一 allow-JIT entitlement。打包的是这个已签名
compiler，不能只验证带签名的临时副本。此步骤不是 Developer ID 签名或 notarization，
不需要签名凭据。

打包前，每个 host 都记录 `ckc --version --verbose` 与 `ckc licenses`，实际执行
`ckc run` 和 `ckc build --kind executable` 生成的 standalone executable，并运行
generated-artifact、compiler dependency 与 JIT memory-permission audit。Linux 与
Windows release 不得保留 dynamic non-system C++ runtime；Darwin 依赖只能解析到
Apple system library。macOS 还要在 hardened runtime 下验证唯一的
`com.apple.security.cs.allow-jit` entitlement。Audit 显式提取 XML 并比较 canonical binary
plist，不解析随系统版本变化的 `codesign`/`plutil` 人类可读输出。JIT audit 必须验证与 runtime capability
一致的 Darwin W^X 路径：per-thread `MAP_JIT`，或在不支持 per-thread 时用页级
RW/NX-to-RX/R-NX finalization；永不接受 RWX fallback。Darwin AOT/ORC object 统一使用 PIC
与 Small code model；未优化的 internal call 必须检查 absolute executable-text relocation。
Standalone executable 验证 dyld 对 `LC_MAIN` 的普通 C-ABI 调用及精确 exit/stdio 行为。Tag run 必须在任何 artifact job
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
asset 不覆盖。`v0.15.0` tag 保留为未发布的历史 source snapshot；`v0.15.1` 是 0.15 系列
首个正式发布版本。后续缺陷必须通过新的 patch version 修复。发布必须 all-or-nothing，包含
六个 archive 和对应的六个 checksum sidecar。
[发布清单](release-checklist.md)是必须完成的 sign-off record。
