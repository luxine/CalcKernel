# I25：Windows 全链静态 CRT 与 COFF 依赖闭包 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans；完全行内，禁止子代理。先提交本计划，再 red/green，不跳过原验收。

**Goal:** 修复真实 Windows 链接失败，让 LLVM/LLD、C++ bridge 与 Rust 在所有构建配置下使用一致的静态 release CRT，并补齐 pinned COFF driver 的链接闭包。

**Architecture:** 继续使用既有 bootstrap、manifest、cache verifier 和 build.rs；用共享 PowerShell guard 检查 CMake 实际编译命令以及已安装 COFF archive 的真实 linker directives。Cargo 对两个 MSVC target 默认启用 crt-static，Native build 拒绝覆盖成动态 CRT。保留五个逻辑 LLVM components，另查询 COFF 的 libdriver/windowsmanifest 依赖。

**Tech Stack:** LLVM/LLD/Clang 22.1.8、CMake、MSVC、PowerShell 7、Rust/Cargo 1.90；Unix 编译参数与产品 ABI 不变。

## 已复诊证据

- 原 run `33302635528`、SHA `5895242`、Windows x64 job `99233477598` 在 bootstrap
  完成后、fact-audit 测试执行前链接失败。完整日志 SHA-256：
  `e9788ec1be76ba5a448fac6e01df8224c0f27d76a7ccf6390355ecc2a398d729`；
  上传的 fact-audit 日志 SHA-256：
  `fee21273c39395f6b0dc3d3a3c4ee15c0fc0917b08fdb51e11578b73558bbc68`。
- CMake 两个 profile 均明确警告未使用 `LLVM_USE_CRT_RELEASE`。大量 LLVM objects
  的 `MD_DynamicRelease` 与 bridge 的 `MT_StaticRelease` 触发 LNK2038/LNK2005。
  pinned `llvm/docs/CMake.rst` 和 CMake 当前文档要求
  `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`；旧选项没有设置真实 `/MT`。
- Rust fact-audit link 命令也带 `/defaultlib:msvcrt`；CI 只在最终 release build
  设置静态 Rust CRT，之前的 Native/CLI/test 没有统一该条件。
- 六个 LNK2019 另指向 LibDriver/WindowsManifest。逐项核对 pinned
  `lld/COFF/CMakeLists.txt` 与 `lld/Common/CMakeLists.txt`，实际 llvm-config 查询
  完整 closure 相对当前五组件只新增 `LLVMDTLTO`、`LLVMLibDriver`、
  `LLVMWindowsManifest`；第一项已显式加入，后两项确实遗漏。
- 旧 cache verifier 只证明文件存在、没有 LLVM DLL 及 runtime hashes；它没有证明
  archive CRT。故本轮已保存的 Windows cache 不能作为合格静态输入。保留失败证据，
  修正 recipe 后使用新键，不删除 directives、不强制链接、不回用旧键。

## Task 0：计划先行

**Files:** 本文、`00-master-control.md`、`11-release-candidate-task.md`、
`11-release-candidate-acceptance.md`、`../review/implementation-blockers-01.md`。

- [x] 行内自审：两项独立根因、Rust 边界、缓存内容证明均有闭环，不改变语言设计。
- [ ] `git diff --check`、`cargo +1.90.0 test --locked --test contracts docs::` 通过后，
  单独提交上述文档，之后才改源码或测试。

## Task 1：先锁定错误配置与实际 archive 反例

**Files:** `tests/contracts/native_toolchain.rs`、`tests/contracts/ci.rs`、
`tests/native.rs`、新 `tests/native/static_prefix.rs`。

- [ ] 更新旧的“包含 LLVM_USE_CRT_RELEASE 即成功”断言：要求当前 CMake runtime
  选项、compile_commands 检查先于 build、静态内容校验先于 cache save。
- [ ] 新契约回归要求两个 MSVC Cargo target 的 crt-static、build.rs 的 target-feature
  与 manifest 一致性拒绝、COFF 两个缺失组件进入 libnames/system-libs 同一查询集合。
- [ ] 执行实际 PowerShell guard 的 compile_commands fixture：接受所有 C/C++ 都是
  `/MT`；拒绝 `/MD`、debug CRT、混合、缺失参数、空编译数据库；不把路径中的文字当参数。
- [ ] Native 测试必须使用配置的 pinned Clang（未配置则明确失败，不 skip）和真实
  llvm-ar/llvm-readobj，构造 x64 与 ARM64 的真正 COFF archives。接受 release static，
  拒绝 dynamic、debug、static+dynamic 混合、空/损坏 archive、缺少工具及失败退出码。
  同时覆盖 RuntimeLibrary mismatch 和 DEFAULTLIB 两种实际 directive。
- [ ] 原 default verifier 的 bytes/hash/path fixture 仍然 LLVM-independent：其 Windows
  分支可显式使用 readobj double 只测试原字段/摘要/DLL 边界；不能把 double 作为 CRT
  内容验收。真正 COFF regression 单独归 Native driver，由必跑矩阵执行。
- [ ] 运行 targeted red，保留预期失败原因及日志；不能把编译错误当行为 red。

## Task 2：统一配置并在缓存前检查真实内容

**Files:** `scripts/bootstrap-llvm.ps1`、新 `scripts/validate-msvc-crt.ps1`、
`scripts/validate-llvm-prefix.ps1`、`.github/actions/bootstrap-ckc-llvm/action.yml`、
`native/llvm/manifest.toml`。

- [ ] 配置 `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`、导出 compile_commands；配置
  成功后、耗时 build 前检查全部 C/C++ 的实际参数。删除无效旧 CRT 选项。
- [ ] 共享 guard 用 pinned llvm-readobj 的 `--coff-directives` 读取每个已声明 `.lib`；
  任一 dynamic/debug RuntimeLibrary 或动态 CRT DEFAULTLIB 都拒绝，每个 archive
  至少一个 release-static 证据。检查错误/空输出/missing tool 均 fail closed；不要求
  每个纯汇编 member 都自带 CRT 标记。逐 archive 调用，避免 Windows 命令长度上限。
- [ ] producer 和 cache verifier 都调用同一 guard；Windows manifest 记录
  `msvc_runtime_library = "MultiThreaded"`，但声明不代替实际 bytes 检查。
- [ ] libnames/system-libs 都查询原 components 加 `libdriver`、`windowsmanifest`，
  检查查询 exit code；保留显式 DTLTO、lldCOFF/lldCommon 和原顺序，不使用 all。
  verifier 和 build.rs 均拒绝缺少这三个 COFF 依赖的 Windows manifest。
- [ ] 新 guard 与 cache verifier 加入完整 recipe digest；其余原输入一个不删。不得
  为命中旧缓存而使用 fallback/restore key；真实两架构 Windows prefix 重新构建。

## Task 3：Cargo 与开发契约

**Files:** 新 `.cargo/config.toml`、`build.rs`、`docs/abi/llvm.md`、
`docs/zh-CN/abi/llvm.md`、`docs/guides/getting-started.md`、对应 zh-CN 文件。

- [ ] 两个精确 MSVC target 默认 Rust `-C target-feature=+crt-static`，debug/release/test
  一致；不向 Unix 或 CK 用户产物添加新 flag/依赖。
- [ ] Native MSVC 的 build.rs 在编译 bridge 前要求 manifest 静态 CRT 身份和
  `CARGO_CFG_TARGET_FEATURE` 中的 crt-static；用户覆盖 Rust flags 时给出明确错误。
  不添加 `/NODEFAULTLIB`、`/FORCE`，不把 bridge 改为 `/MD`。
- [ ] 同步双语当前文档：Windows 全链静态配置、source builds 的 flags 覆盖责任、
  prefix 必须用校验器验证；PowerShell/Clang fixture 只属于开发测试，不增加运行依赖。

## Task 4：原门槛全部保留

- [ ] 同组 targeted green、原 cache corruption/DLL tests、完整 default/all-feature、
  Clippy/fmt、release lib/IR/native build、generated/mutation/fact audit/cache tests、
  artifact/JIT/version/licenses 全通过，0 failed/ignored。
- [ ] 行内对抗性复审动态/混合 CRT 是否可绕过、两种架构 COFF closure、guard 调用顺序、
  Cargo 覆盖诊断及 Unix 不变性。新真实阻断先记录再修，不扩大优化设计。
- [ ] 提交验证后的实现与本地证据；以确切新 SHA 做首次完整 schema-6 性能门，保留
  原件和全部原阈值。检查 replay bundle identity，真实输入改变则重新准备，不能复用失配。
- [ ] 原 Windows ARM job 仍在旧 recipe 构建时可继续本地修复，不把它当成已通过，也不
  声称最终会得到合格 CRT cache。保存其终态与日志；新正确 recipe 运行完整十项 CI。
- [ ] 同一最终 SHA 的全部十项 required jobs 通过后才签收 I25/阶段 11；随后执行
  01–11/99 总验收，最终证据提交再过同 SHA 完整 CI，不合并 main。

## 行内计划自审

这是工具链实现缺陷，不是静态/零运行依赖契约反例。配置命令与 archive bytes 分开检查，
避免继续信任被忽略的声明；Rust target feature 解决下游同样的 CRT 分叉。真正 COFF
回归可跨 host 执行，但不能替代两架构 MSVC 最终链接与依赖审计。现有逻辑五组件与
host-only 产品边界不变，额外两项仅服务已选 COFF driver。没有降低性能或安全门槛。
