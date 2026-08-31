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
- [x] `git diff --check`、`cargo +1.90.0 test --locked --test contracts docs::` 通过后，
  单独提交上述文档，之后才改源码或测试。
  初始计划提交 `c17e1bf`，host-only fixture 修订提交 `45a88da`；docs 16 passed。

## Task 1：先锁定错误配置与实际 archive 反例

**Files:** `tests/contracts/native_toolchain.rs`、`tests/contracts/ci.rs`、
`tests/native.rs`、新 `tests/native/static_prefix.rs`。

- [x] 更新旧的“包含 LLVM_USE_CRT_RELEASE 即成功”断言：要求当前 CMake runtime
  选项、compile_commands 检查先于 build、静态内容校验先于 cache save。
- [x] 新契约回归要求两个 MSVC Cargo target 的 crt-static、build.rs 的 target-feature
  与 manifest 一致性拒绝、COFF 两个缺失组件进入 libnames/system-libs 同一查询集合。
- [x] 执行实际 PowerShell guard 的 compile_commands fixture：接受所有 C/C++ 都是
  `/MT`；拒绝 `/MD`、debug CRT、混合、缺失参数、空编译数据库；不把路径中的文字当参数。
- [x] Native 测试必须使用配置的 pinned Clang（未配置则明确失败，不 skip）和真实
  llvm-ar/llvm-readobj，构造当前 host 架构的真正 COFF archives（六 host 矩阵合起来覆盖
  x64 与 ARM64；pinned prefix 按设计只编译一个 host backend）。接受 release static，
  拒绝 dynamic、debug、static+dynamic 混合、空/损坏 archive、缺少工具及失败退出码。
  同时覆盖 RuntimeLibrary mismatch 和 DEFAULTLIB 两种实际 directive。
- [x] 原 default verifier 的 bytes/hash/path fixture 仍然 LLVM-independent：其 Windows
  分支可显式使用 readobj double 只测试原字段/摘要/DLL 边界；不能把 double 作为 CRT
  内容验收。真正 COFF regression 单独归 Native driver，由必跑矩阵执行。
- [x] 运行 targeted red，保留预期失败原因及日志；不能把编译错误当行为 red。

## Task 2：统一配置并在缓存前检查真实内容

**Files:** `scripts/bootstrap-llvm.ps1`、新 `scripts/validate-msvc-crt.ps1`、
`scripts/validate-llvm-prefix.ps1`、`.github/actions/bootstrap-ckc-llvm/action.yml`、
`native/llvm/manifest.toml`。

- [x] 配置 `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`、导出 compile_commands；配置
  成功后、耗时 build 前检查全部 C/C++ 的实际参数。删除无效旧 CRT 选项。
- [x] 共享 guard 用 pinned llvm-readobj 的 `--coff-directives` 读取每个已声明 `.lib`；
  任一 dynamic/debug RuntimeLibrary 或动态 CRT DEFAULTLIB 都拒绝，每个 archive
  至少一个 release-static 证据。检查错误/空输出/missing tool 均 fail closed；不要求
  每个纯汇编 member 都自带 CRT 标记。逐 archive 调用，避免 Windows 命令长度上限。
- [x] producer 和 cache verifier 都调用同一 guard；Windows manifest 记录
  `msvc_runtime_library = "MultiThreaded"`，但声明不代替实际 bytes 检查。
- [x] libnames/system-libs 都查询原 components 加 `libdriver`、`windowsmanifest`，
  检查查询 exit code；保留显式 DTLTO、lldCOFF/lldCommon 和原顺序，不使用 all。
  verifier 和 build.rs 均拒绝缺少这三个 COFF 依赖的 Windows manifest。
- [x] 新 guard 与 cache verifier 加入完整 recipe digest；其余原输入一个不删。不得
  为命中旧缓存而使用 fallback/restore key。真实两架构 Windows prefix 的重新构建
  属于 Task 4 远程门，当前尚未签收。

## Task 3：Cargo 与开发契约

**Files:** 新 `.cargo/config.toml`、`build.rs`、`docs/abi/llvm.md`、
`docs/zh-CN/abi/llvm.md`、`docs/guides/getting-started.md`、对应 zh-CN 文件。

- [x] 两个精确 MSVC target 默认 Rust `-C target-feature=+crt-static`，debug/release/test
  一致；不向 Unix 或 CK 用户产物添加新 flag/依赖。
- [x] Native MSVC 的 build.rs 在编译 bridge 前要求 manifest 静态 CRT 身份和
  `CARGO_CFG_TARGET_FEATURE` 中的 crt-static；用户覆盖 Rust flags 时给出明确错误。
  不添加 `/NODEFAULTLIB`、`/FORCE`，不把 bridge 改为 `/MD`。
- [x] 同步双语当前文档：Windows 全链静态配置、source builds 的 flags 覆盖责任、
  prefix 必须用校验器验证；PowerShell/Clang fixture 只属于开发测试，不增加运行依赖。

## Task 4：原门槛全部保留

- [x] 同组 targeted green、原 cache corruption/DLL tests、完整 default/all-feature、
  Clippy/fmt、release lib/IR/native build、generated/mutation/fact audit/cache tests、
  artifact/JIT/version/licenses 全通过，0 failed/ignored。
- [x] 行内对抗性复审动态/混合 CRT 是否可绕过、两种架构 COFF closure、guard 调用顺序、
  Cargo 覆盖诊断及 Unix 不变性。新真实阻断先记录再修，不扩大优化设计。
- [ ] 提交验证后的实现与本地证据；以确切新 SHA 做首次完整 schema-6 性能门，保留
  原件和全部原阈值。检查 replay bundle identity，真实输入改变则重新准备，不能复用失配。
  实现已提交为 `d424270`。首次报告因外部负载下的多通道采样失稳失败，完整保留；
  先复诊并记录条件后，在同 SHA/同协议下唯一一次 qualification 通过原 checker。
  详见阶段 11 acceptance 的失败与复验证据；未把失败原件改写成通过。
- [ ] 原 Windows ARM job 仍在旧 recipe 构建时可继续本地修复，不把它当成已通过，也不
  声称最终会得到合格 CRT cache。保存其终态与日志；新正确 recipe 运行完整十项 CI。
  已知不合格 recipe 不再值得等待复用；新 dispatch 若按既有 concurrency 取消旧运行，
  如实保留 cancelled，不能写成自然完成或测试成功。
  实际旧运行已终止为 cancelled：七项 success、Windows x64 failure、Windows ARM
  与 Darwin x64 cancelled；取消后的完整日志摘要记录在阶段 11 acceptance。
  新 `33316188869` 绑定 `d424270`；quality、AArch64 performance、Linux ARM host、
  Darwin ARM host、Linux x64 host、native integration、x86-64 performance 已通过，
  Darwin x64 也已通过；其余两架构 Windows 尚在构建或验收；
  此清单项仍待新完整 CI 的实际结果，不能因已保存旧日志而提前关闭。
- [ ] 同一最终 SHA 的全部十项 required jobs 通过后才签收 I25/阶段 11；随后执行
  01–11/99 总验收，最终证据提交再过同 SHA 完整 CI，不合并 main。

## 行内计划自审

测试准备阶段修订：第一次真实 Clang fixture 在 ARM host 请求 x64，被 host-only LLVM
拒绝；该错误日志保留，不算 CRT 行为 red。测试按本机已编译 backend 生成同架构 COFF，
不扩大 LLVM build targets、不换未固定的 Clang。两架构覆盖仍由完整必跑矩阵证明，
实际两 Windows MSVC 链接门不变。这修正了测试计划与既有 host-only 契约的冲突。

这是工具链实现缺陷，不是静态/零运行依赖契约反例。配置命令与 archive bytes 分开检查，
避免继续信任被忽略的声明；Rust target feature 解决下游同样的 CRT 分叉。真正 COFF
回归可跨 host 执行，但不能替代两架构 MSVC 最终链接与依赖审计。现有逻辑五组件与
host-only 产品边界不变，额外两项仅服务已选 COFF driver。没有降低性能或安全门槛。

本地执行证据：default 475 / all-feature 606（Native 102），release lib 53 / IR 58、
generated 3 / mutation 10 / fact audit 7 / verifier-cache 5 / docs 16 全部通过，0 failed/ignored。
两种 Clippy、fmt/diff、Native release build、actual compiler 签名/依赖、artifact/JIT audit
和 Unix prefix verifier 通过。真实 COFF 新测试为 3 项；细节与日志摘要见 review 的 I25。
本地通过不是两架构 MSVC 验收，最终十项 CI 与首次新 SHA 性能仍待签收。

## Task 5：真实 Windows Native execution 闭包（I26）

候选 run `33316188869` 的 x64 job `99269971157` 已证明 bootstrap、实际 `/MT`
compile commands、安装后 archive 检查、oracle profile 和 pre-LLVM fact audit 能完成；但
`Run required native suite` 为 62 passed / 30 failed。完整日志 SHA-256 为
`2315bc4d21c60ea36ff12085864733a3879085102db34bdfc5086602ff89f0ba`，fact-audit
artifact 原文件 SHA-256 为
`27c2a74b0ed7af65bfea3706d849ac3bf01725a1e5f6ebe2ce8a8ecf289d780b`。这使 I25
继续保持未签收；已经通过的八项不能与后续 SHA 拼接。

### 5.1 复诊与不变量

- COFF driver 实际选择正确，但 shared/executable 的公共尾部仍统一追加 Unix `-o`；
  `lld-link` 明确警告忽略它并把输出路径当输入文件。Windows 必须使用单参数
  `/out:<path>`，Darwin/ELF 的 `-o <path>` 保持不变。
- COFF x64 JITLink 为 MSVC C runtime object 的 `.pdata`/image-relative relocation
  生成外部 `__ImageBase`；当前禁用任意 process-symbol 搜索且没有内部定义，故所有 JIT
  消费路径以 `Symbols not found: [ __ImageBase ]` 失败。不得开放整进程符号、切回
  RuntimeDyld、删除 `.pdata` 或放宽 JIT audit。
- 两个 IR 测试把 `define i32` 当成跨平台文本；Windows 正确的 external definition 为
  `define dllexport i32`。修复断言只接受 `define` 行中可选平台 export storage class，仍需
  精确返回类型/参数与 internal implementation，不能删测试。
- CK 用户产物仍只链接五个原 runtime objects；PE/COFF LLD 自己拥有最终 image base。
  额外对象只属于 x86-64 Windows JIT 内部支持面，不进入 static/shared/executable
  artifact，也不改变 Runtime ABI、Native ABI、语言语义或公开符号表。

### 5.2 计划先行与 TDD

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改
`tests/contracts/native_toolchain.rs`、`tests/native/abi.rs`、`tests/native/llvm_ir.rs`、
`native/runtime/windows/jit_image_base.c`、`scripts/bootstrap-llvm.ps1`、
`scripts/validate-llvm-prefix.ps1`、`build.rs`、`src/backend/native_runtime.rs`、
`src/backend/llvm/{ffi,jit}.rs`、`native/bridge/ckc_llvm.cpp` 及双语 LLVM ABI 文档。

- [x] 先提交本计划及现有失败证据，提交前通过 diff/docs 16；不得把生产修复混入计划提交。
- [x] red 先锁定 COFF `/out:` 分支、x64-only JIT support manifest/hash/cache 校验、五个
  artifact runtime objects 与内部 JIT support 分离、FFI 对支持对象数量 fail-closed，以及
  Windows `dllexport` IR 行。编译错误、缺少本机 Windows SDK 或 source-string 假阳性不算 red。
- [x] 新 `jit_image_base.c` 只定义 JIT 私有 anchor 的 COFF `__ImageBase`，无函数、CRT
  调用、默认库或公开 CK entry。bootstrap 只为 `x86_64-pc-windows-msvc` 编译/安装/散列；
  build.rs 和 cache verifier 校验精确文件、hash、target 与路径，ARM64 和 Unix 拒绝伪字段。
- [x] JIT x64 在同一 `MapperJITLinkMemoryManager`/JITDylib 中先加入已验证 anchor，再加入
  五个 runtime objects 与 program object；512 MiB reservation hard bound 足以容纳该固定
  七对象闭包并保持 32-bit image-relative relocation 可表示。不得把 anchor 传给 LLD
  artifact link，也不得给 generated CK code 开放额外 host symbol。
- [x] COFF shared/executable driver 使用 `/out:<path>`；ELF/Mach-O 命令逐字保持原形式。
  export allowlist、`/nodefaultlib`、`/noentry`/entry、import library 和输出格式验证均保留。

### 5.3 验证与远程闭环

- [x] 本地 targeted、default/all-feature、两种 Clippy、fmt/diff、release lib/IR/native、
  generated/mutation/fact-audit/cache/docs、artifact/JIT/version/licenses 与原 schema-6
  性能门全部通过；不得降低计数、阈值或把 Windows-only 门标成 skip 成功。
- [x] 保存 `33316188869` Windows ARM64 的自然终态和完整日志；该 run 已有 x64 failure，
  无论 ARM 结果如何都不能签收或 rerun 成新代码证据。
- [ ] 以修复提交的新 SHA 重新执行全部十项 required CI；Windows x64/ARM64 都须通过
  bootstrap、fact audit、完整 Native/CLI、release static build、compiler dependency、
  artifact/JIT audit，且日志无 `unknown argument '-o'`、`__ImageBase`、LNK2038/2005/2019。
  同 SHA 完整通过后才关闭 I26/I25/阶段 11，再执行 01–11/99 总验收和最终 docs-only
  SHA 的完整十项 CI。

## Task 5 行内对抗性自审

anchor 不是伪造 PE 或开放 process search：它给 JITLink 的 image-relative relocation 一个
与固定对象集同 reservation 的内部基准；该对象不被 AOT LLD 消费。将其限定为 COFF x64
也避免与 LLVM 22.1.8 的 ARM64 RuntimeDyld 路径重复定义。manifest、cache 和 build.rs
必须共同绑定其 bytes，避免缓存中缺失/串架构。最终正确性仍由两个真实 MSVC host 的完整
Native/JIT/artifact 门证明；本地 source contract 不能替代远程执行。此修订保留 JITLink、
W^X、五个公开 runtime objects、静态 CRT、无默认库和所有原始验收门，因此没有通过任务而
降低设计标准。

## Task 6：COFF ARM64 RuntimeDyld 符号责任闭包（I27）

旧候选 run `33316188869` 的 Windows ARM64 job `99269971150` 已自然结束为 failure，
没有被后续 dispatch 取消。它完成 release/oracle bootstrap、静态 CRT/archive 校验和
pre-LLVM fact audit 7/7，进入 Native suite 后出现 18 个明确失败标记，随后以
`0x80000003` 终止，未生成伪造的汇总通过行。完整日志 SHA-256 为
`0e9351c157354ea90a4cb8908d5ac524875966abc6a351bc92630162263ab67f`；fact-audit
artifact ID `9737795689`，zip / 原文件 SHA-256 分别为
`2752149aea74bb5ecde01b6823437e89ee334e457e99ce5674b44bc0d3024c78` /
`1316726ad12ae778e9e5ecaa5c4cb58b073539dbd4a861f7cf42b0cc478f8250`。

其中 artifact、sanitizer、executable、differential 与 ABI 文本失败已由 I26 的
COFF `/out:` 和 Windows `dllexport` 修复覆盖；ARM64 cache/run/JIT 子进程及父进程则
稳定触发 LLVM 断言：`Resolving symbol with incorrect flags`（pinned `Core.cpp:2803`）。
这不是 x64 `__ImageBase` 问题，也不能由 x64-only anchor 修复。

### 6.1 复诊与不变量

- CK 为保留 audited `CkcAuditedSectionMemoryManager`，在 COFF ARM64 上替换了 LLJIT 的
  默认 object-layer creator；自定义 creator 构造 bare `RTDyldObjectLinkingLayer` 后直接
  返回，漏掉 pinned LLVM 22.1.8 的 LLJIT 默认 COFF 配置。
- [pinned LLJIT creator](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/lib/ExecutionEngine/Orc/LLJIT.cpp)
  对 COFF 同时调用 `setOverrideObjectFlagsWithResponsibilityFlags(true)` 和
  `setAutoClaimResponsibilityForObjectSymbols(true)`；前者把 RuntimeDyld resolved flags
  与 materialization responsibility 已声明 flags 对齐，后者认领 COFF weak/COMDAT
  等额外 object symbols。对应行为见 pinned
  [RTDyldObjectLinkingLayer.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/lib/ExecutionEngine/Orc/RTDyldObjectLinkingLayer.cpp)
  与断言所在的 [Core.cpp](https://github.com/llvm/llvm-project/blob/llvmorg-22.1.8/llvm/lib/ExecutionEngine/Orc/Core.cpp)。
- 修复仅限既有 `aarch64 + COFF` RuntimeDyld 分支，必须继续使用 audited memory manager、
  process-symbol search disabled、固定 allowlist、W^X 审计和既有 backend identity。
  不得关闭 LLVM assertions、吞掉错误、切换 ARM64 到未经支持的 JITLink、加入 x64 anchor、
  开放整进程符号或把 ARM64 JIT tests 标成 skip。

### 6.2 计划先行与 TDD

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改
`tests/contracts/native_toolchain.rs`、`native/bridge/ckc_llvm.cpp` 和双语 LLVM ABI 文档。

- [x] 先提交 I27 复诊、真实远程证据和本计划；`git diff --check` 与 docs 16 通过，
  生产实现不得混入该提交。
- [x] red contract 必须在 ARM64 COFF creator 的同一局部同时证明 audited memory manager、
  object-flags override 与 object-symbol auto-claim；仅在注释或无关分支出现字符串不能通过。
- [x] 实现先构造有类型的 `RTDyldObjectLinkingLayer`，对该实例设置上述两个官方 COFF
  选项后再上转型返回 `ObjectLayer`。x64 JITLink 与所有 Unix creator 不得改变。
- [x] 同步英中 LLVM ABI 当前文档，说明这是官方 COFF responsibility contract 的恢复，
  不把 RuntimeDyld 描述成新的通用后端或降低 JIT 隔离保证。

### 6.3 验证与远程闭环

- [ ] 先保留 targeted red，再通过 targeted contract；随后重跑 Task 5 的全部本地门和
  原 schema-6 性能门，计数/阈值/语料不变。
- [ ] 在修复提交的精确 SHA 上重新 dispatch 十项 required CI。Windows ARM64 必须完成
  Native/CLI、cache/run/JIT、release artifact 与 audit，日志不得再出现 incorrect-flags
  assertion 或 `0x80000003`；Windows x64 同时证明 I26，其他八项不可从旧 SHA 拼接。
- [ ] 只有该 SHA 十项全绿，才允许关闭 I27/I26/I25/阶段 11；此后仍按总控文档执行
  01–11/99 总验收，并使最终 docs-only 交付 SHA 再通过同一完整十项矩阵。

## Task 6 行内对抗性自审

两项 setter 不是凭失败文本猜测的新策略，而是自定义 creator 必须手工恢复的 pinned LLJIT
默认 COFF 行为；它们直接对应 resolved flags 断言和 COFF 额外符号责任。修复面被限制在
现有 ARM64 RuntimeDyld 分支，不影响 x64 JITLink、AOT artifacts、ABI 或语言语义。真实
Windows ARM64 execution 仍是最终证明，source contract 只防止再次漏配，不能替代远程门。
复审未发现需要改变原安全模型或降低验收标准的阻断项。

## Task 7：Windows x64 JIT object slice 类型闭包（I28）

修复 SHA `7b03f76e1139ec91a5962ca18e696c2c127604c2` 的完整 run
`33332458652` 在其他八项 success 后，Windows x64 job `99313407116` 自然结束为
failure。release/oracle bootstrap、77 个 MSVC archive 静态 CRT 检查、x64 JIT support
object 构建、prefix 验证和两条 cache save 均完成；首次编译 fact-audit target 时，
`src/backend/native_runtime.rs` 触发 E0277/E0308，未进入 Native suite。完整日志
SHA-256 为 `5265e7791eef8994a24209daab993566d2f84d2f58ef40a26515604ed5b801a9`；
失败 artifact ID `9741692125`，zip / 原文件 SHA-256 分别为
`f262aa42e985645d8362cf92569fbb3009a853288f721155a0201012cb571c2d` /
`3e8f01a73123facfcc3cfad3e977cf31153517144eccf187b23464178521be60`。

### 7.1 复诊与不变量

- `embedded_jit_objects` 的公开内部契约已经声明返回 `Vec<&'static [u8]>`，但局部
  `objects` 使用无类型注解的 `Vec::with_capacity(6)`。Windows x64 cfg 下第一个 `push`
  是 `include_bytes!(CKC_RUNTIME_JIT_SUPPORT)`，其具体类型为 `&[u8; 621]`，Rust 因而先把
  容器窄化成 `Vec<&[u8; 621]>`；随后 `extend(embedded_runtime_objects())` 的元素类型
  `&[u8]` 无法满足 `Extend`，返回类型也无法补救已经完成的方法调用推断。
- Linux、Darwin 和 Windows ARM64 不编译这个 x64-only `push`，所以既有本机、Unix 和
  ARM64 入口不能证明该分支可编译。这是生产 cfg coverage 缺口，不是 LLVM、COFF、静态
  CRT 或 artifact 内容错误。
- 修复必须只给局部容器一个显式 `Vec<&'static [u8]>` 类型，使 array reference 在 `push`
  边界发生标准 unsize coercion；不得复制 object bytes、改变 anchor-first + 五 runtime
  objects 的顺序/数量、改动 bootstrap/cache manifest、ABI、ORC、process isolation、W^X
  或任何测试/性能阈值。

### 7.2 计划先行与 TDD

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改
`tests/contracts/native_toolchain.rs` 与 `src/backend/native_runtime.rs`。

- [ ] 先提交 I28 远程原件、复诊、不变量和本计划；docs 16 与 `git diff --check` 通过，
  生产实现不得混入该提交。
- [ ] 在既有 Windows native execution contract 中先增加局部 slice-collection 类型断言，
  对旧生产源码实际观察 targeted red；不能只在注释或测试 fixture 里出现目标字符串。
- [ ] 最小实现只为 `objects` 增加 `Vec<&'static [u8]>` 注解。targeted contract、default
  contract suite 与本机 native-feature 编译必须 green，且现有 object count/order 断言保留。

### 7.3 验证与远程闭环

- [ ] 重跑 Task 6 的全部本地门与原 schema-6 性能门；计数、语料、阈值和工具链 identity
  不变。只有完整通过后才允许提交/推送实现。
- [ ] 在新精确 SHA 上重新执行十项 required CI。Windows x64 必须完成 fact/Native/CLI、
  static compiler、实际 executable/library/JIT 和发布审计；Windows ARM64 必须同时完成
  I27 的 incorrect-flags 路径。不得拼接 `7b03f76` 的八项成功。
- [ ] 新 SHA 十项全绿后才能关闭 I28/I27/I26/I25/阶段 11，并继续 01–11/99 总验收与
  最终 docs-only SHA 的第二轮完整矩阵。

## Task 7 行内对抗性自审

失败由 Rust 诊断直接给出具体推断链，显式返回类型不足以反向约束先发生的 `push`；因此
局部 collection 注解是最小且充分的修复。它不改变任何 object bytes、顺序或执行语义，
同时把跨 cfg 编译意图变成稳定源码契约。文本 contract 只能防回归，实际 Windows x64
编译/执行仍是必要验收；完整十项矩阵和既有 ARM64 ORC 门均不降低。复审未发现需扩张
设计或调整规范/ABI 的阻断项。
