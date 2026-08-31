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

- [x] 先保留 targeted red，再通过 targeted contract；随后重跑 Task 5 的全部本地门和
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

- [x] 先提交 I28 远程原件、复诊、不变量和本计划；docs 16 与 `git diff --check` 通过，
  生产实现不得混入该提交。
- [x] 在既有 Windows native execution contract 中先增加局部 slice-collection 类型断言，
  对旧生产源码实际观察 targeted red；不能只在注释或测试 fixture 里出现目标字符串。
- [x] 最小实现只为 `objects` 增加 `Vec<&'static [u8]>` 注解。targeted contract、default
  contract suite 与本机 native-feature 编译必须 green，且现有 object count/order 断言保留。

### 7.3 验证与远程闭环

- [x] 重跑 Task 6 的全部本地门与原 schema-6 性能门；计数、语料、阈值和工具链 identity
  不变。只有完整通过后才允许提交/推送实现。
- [ ] 在新精确 SHA 上重新执行十项 required CI。Windows x64 必须完成 fact/Native/CLI、
  static compiler、实际 executable/library/JIT 和发布审计；Windows ARM64 必须同时完成
  I27 的 incorrect-flags 路径。不得拼接 `7b03f76` 的八项成功。
- [ ] 新 SHA 十项全绿后才能关闭 I28/I27/I26/I25/阶段 11，并继续 01–11/99 总验收与
  最终 docs-only SHA 的第二轮完整矩阵。

### 7.4 本地执行证据（远程未签收）

- docs-first 提交 `bc009e5` 只含 I28 原件、复诊与计划。随后 contract 在旧生产源码上
  实际为 0 passed / 1 failed，失败信息精确要求显式 slice-typed collection；最小实现
  只把局部声明改为 `let mut objects: Vec<&'static [u8]> = Vec::with_capacity(6);`，同一
  targeted contract 随后 1/1 green。既有 anchor-first、五 runtime objects、容量与返回
  类型断言均保留。
- 完整 default 477 / all-feature 608（Native 102）、release lib 53 / IR 58、generated 3 /
  mutation 10 / fact audit 7 / verifier-cache 5 / docs 16、两种 Clippy、fmt/diff、artifact
  fixture 5、compiler/artifact/JIT/version/licenses 全部通过，0 failed/ignored。default /
  all-feature 日志 SHA-256 为
  `d2d4ba35f95172a21581644f681b64af46b92e68f8d3f273f7055656ec594b20` /
  `f45b697fb89b6e0eb447d2dcf59b8544dc9e7327726ddecc7ac7b1c65835a4fc`。
- 旧本机 prefix 因 manifest 未列出实际存在的 `LLVMDTLTO` 被当前 verifier 正确拒绝，未
  手改清单或绕过门。按当前配方重新构建 release/oracle 后，manifest SHA-256 分别为
  `8a0d25cdcd729cd35be139d9f3b571d3a0769a380d1fce1e9731292119dc290c` /
  `b073daad34f4dfd5055614c7893b42c38f875cb54198b528729247dd3d13f934`，两个 profile 均由
  `validate-llvm-prefix.ps1` 通过；实际 release compiler 也报告前一摘要并通过全部发布审计。
- 第一次完整 schema-6 报告保留为失败：runtime 四组与 proof 通过，但共享主机刚完成
  4361-target LLVM 构建并持续出现多核 Node/index/FSEvents 负载，六个 optimizer case
  相对此前同机合格结果全面变慢；`example-dijkstra` 为 `1328333 / 350000 = 3.7952x`，
  超过原 3x individual 门。report / benchmark / checker SHA-256 为
  `0c4c5420d664028e8a0341a754f938aa45ff077b63f6a8f21a0b3efafa8d38bc` /
  `837026db6a75bcd22eb01fee27a232b40f9b529db016b83329cf711809d5189d` /
  `2d539d81e4bb79ed56a560b31d629be81d7a0a3148e7716bf9c40e85b0a23718`。
- 沿用既有不降门槛资格规则，只读等待到连续六个当前样本 idle 74%–84% 且进程快照无
  高负载构建/索引后，执行唯一一次同工作树、同 bundle、同参数 qualification。原 checker
  exit 0：unchecked Clang/replay `0.9993 / 1.0002`，checked `1.0014 / 1.0011`，proof
  `0.9965`，optimizer suite `1.0929`，Dijkstra `1.9954x`；24 measurement + 8 replay
  artifacts 完整。report / benchmark / checker SHA-256 为
  `9618e947dd66a31aa0258691117087d3e040392fc9c4ed64710e3a0d5496a682` /
  `e42894b8ed4e8e99a0f6e58be5649e2901eda7039a1d9d4ef0bcbb2d296bd3c9` /
  `79181453144d5862641e70aa0a710fd27b928cc4562eac2f746c2424e83f8a0c`。首次失败原件未
  删除或覆盖，没有第三次计时；新提交精确 SHA 的十项远程矩阵仍是必要验收。

## Task 7 行内对抗性自审

失败由 Rust 诊断直接给出具体推断链，显式返回类型不足以反向约束先发生的 `push`；因此
局部 collection 注解是最小且充分的修复。它不改变任何 object bytes、顺序或执行语义，
同时把跨 cfg 编译意图变成稳定源码契约。文本 contract 只能防回归，实际 Windows x64
编译/执行仍是必要验收；完整十项矩阵和既有 ARM64 ORC 门均不降低。复审未发现需扩张
设计或调整规范/ABI 的阻断项。

## Task 8：Windows ARM64 host conformance 与 cache touch 闭包（I29）

旧精确 SHA `7b03f76e1139ec91a5962ca18e696c2c127604c2` 的 run `33332458652`
最终 completed/failure。Windows ARM64 job `99313407132` 自然完成当前 recipe 的
release/oracle bootstrap，两个 `c8b5101e…` cache 均保存并通过 prefix 验证；pre-LLVM
fact audit 7/7 后，required Native suite 为 87 passed / 5 failed / 0 ignored。完整 job
日志 SHA-256 为 `7b13024e4f5177674a999f03ce8bbe2cb438eece7b90c98480ffe5ee80b60720`；
fact artifact ID `9742713855`，zip / 原文件 SHA-256 为
`bc023be4906d0a3bedf39d3b6fd32ed1599c48a69869211602669dd73ffaea85` /
`dee966674a5e187610d392b266a03f6a2746e930f3dc39ba13e26d564cc975b4`。

### 8.1 复诊与不变量

- I27 的 `Resolving symbol with incorrect flags` 与 `0x80000003` 均未复现；cache/run/JIT、
  complete object graph、checked modes、memory audit、standalone executable 与 generated
  differential 等路径实际通过。ARM64 RuntimeDyld responsibility 修复有效，但该旧 SHA
  仍因后续 host conformance 缺口不能签收。
- `native_llvm_should_hide_internal_signatures_behind_host_c_abi_thunks` 仍把 AArch64 external
  definition 写死为 `define [2 x i64]`；Windows 正确插入 `dllexport`。只允许像已有 checked
  ABI 断言一样匹配 `define` 行中的返回 shape/符号，不能移除 storage class 或降低 ABI shape。
- `native_jit_should_use_jitlink_on_macos_aarch64` 名称限定 macOS、实现却在六 host 都执行并
  硬编码 JITLink；通用 ownership test 也有相同问题。两者必须改为消费已冻结的六 host
  policy：仅 Windows ARM64 为 `RuntimeDyldCoffAarch64`，其余 host 为 JITLink；不得改回
  ARM64 JITLink，也不得用 cfg 跳过 Windows 测试而把 Native 计数从 92 降到 91。
- Windows C differential oracle 使用 Clang `-shared` 但没有把从同一 MIR 得到的 exports
  传给 COFF linker，故首先在 oracle `GetProcAddress("scalar")` 失败，尚未比较 Native
  library。fixture 必须对 Windows 精确追加同一 exports 集的 `/export:<name>`；不能修改
  CK Native export 实现、硬编码少于完整集合或把缺符号当 skip。
- cache warm-hit 使用 `File::set_times` 更新 LRU mtime，但 Windows entry handle 只有
  `GENERIC_READ`；`FILE_WRITE_ATTRIBUTES` 缺失使调用静默失败。修复只给 no-follow handle
  增加属性写权限，同时保留 read data、`FILE_FLAG_OPEN_REPARSE_POINT`、owner-only root、
  entry bytes 与 best-effort cache-hit 语义。不得延长测试 sleep、重写 cache entry 或把
  touch 失败升级成 CK 程序执行失败。

### 8.2 计划先行与现有真实 red

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改 `tests/native/abi.rs`、
`tests/native/ownership.rs`、`tests/native/differential.rs`、`src/cli/cache/store.rs` 与必要的
production-source contract。

- [x] 先提交自然终态、完整 hashes、五项分类、不变量与本计划；docs 16 / diff 通过，
  不混入实现或测试修订。
- [x] 以 job `99313407132` 的 87/5 作为真实 Windows ARM64 red。修复 ABI/ownership/oracle
  fixture 时保留原结构和失败可诊断性；cache 必须修生产 handle，不能只改时间等待断言。
- [x] 最小 green：ABI line matcher 容纳合法 `dllexport`；两个 ownership tests 都重命名/
  改写为冻结 host policy 断言并继续在六 host 执行；oracle export list 从同一 MIR 派生并
  仅在 COFF 传给 linker；Windows cache open 使用 `GENERIC_READ | FILE_WRITE_ATTRIBUTES`
  且继续 no-follow。

### 8.3 验证与远程闭环

- [x] targeted/default/all-feature、两种 Clippy、fmt/diff、release lib/IR/native、全部独立
  小门、artifact/compiler/JIT/version/licenses 与 schema-6 原门全绿；测试计数、语料和
  阈值不降。本项只签收本地与计时门；Windows-only green 仍由真实 ARM64/X64 host 证明。
- [ ] 将 I29 修复叠加到已本地验证的 I28 提交后，以新的精确 SHA dispatch 全十项矩阵。
  Windows ARM64 必须 92/92 Native + CLI + release/audits 全绿；Windows x64 同时证明 I28
  编译闭包；其他八项不能从 `7b03f76` 拼接。
- [ ] 同 SHA 十项全绿后才关闭 I29/I28/I27/I26/I25/I23/阶段 11，并进入 01–11/99 总验收。

### 8.4 实现与本地证据

- docs-first 提交 `fd594d9587cee8cf5c69d797fde0dab2940976c4` 只含旧 run 自然终态、
  五项复诊、修订边界与计划。随后 Windows cache production-source contract 在旧源码上
  实际 0/1 red，最小 access-mask 修订后 1/1 green；red / green 日志 SHA-256 为
  `1430f8d2266322b8c8fdeaf97e5be04f6591453a57f798d7134eba12cf34f4dd` /
  `7c23eefff9612a796cd95474e7779466afb96d1ab9f285560c5edc566c5e2824`；最终 contract 还在
  行内复审后锁定三个 Windows 常量的精确数值，避免仅有常量名称的假绿。
- 唯一生产改动把 Windows no-follow cache entry handle 的 access mask 从隐式 read 改为
  `GENERIC_READ | FILE_WRITE_ATTRIBUTES`；没有请求 generic write、改 cache bytes/格式、
  延长 sleep 或改变 best-effort 失败语义。ABI line matcher 仍要求精确 small aggregate
  shape/sret；两个 ownership tests 继续在六 host 执行；C oracle exports 从同一 MIR
  一次派生并同时供 oracle linker 与 Native linker 使用。
- 完整 default 478 / all-feature 609（Native 102）、单独 Native 102、release lib 53 / IR 58、
  generated 3 / mutation 10 / fact audit 7 / verifier-cache 5 / docs 16 全部 0 failed/ignored；
  两种 Clippy、fmt/diff、artifact fixture 5、compiler/artifact/JIT/version/licenses 与两个
  prefix verifier 全绿。default / all-feature / Native / 独立小门 / release-audits 日志
  SHA-256 分别为 `9d0ac7d1ee8b66d1f1ffb515abc839da399edd36aa85230e5cec4cc8eaad3571` /
  `92f5c2e5f3a5af24e7e64755f8941d14e00cd3d4e687d5394abe1b71f7bf8642` /
  `da8cffc4fdba087118ba2a47a8e3565a3dc93c1be69f8445d5591c00b205425f` /
  `65fda20fb4b36222c18215420446ed1c8cce6cca55f04a75991777ac65a8bd6d` /
  `c01231d71e997cc86f8baea0ea03c38c631f34f32e1d39f8614676bf0278e546`。
- 第一个正式 preflight 的六个 CPU 样本为 72%–86% idle，但末尾进程快照捕获刚启动的
  `project-index generate`，因此没有启动 benchmark；该失败前检原件 SHA-256 为
  `53fe06e727240648d456df4a62933b1fca5e21c90425b1f69ce93ac53b8af4bb`。随后只读等待到
  六个当前样本为 76%–85% idle 且首尾均无高负载编译/索引，资格前检 SHA-256 为
  `b4611a64966b5f1d17dec47dcdfda6c127aceaf56da683462f0e9303929fdd67`。
- 只执行一次 schema-6 benchmark；同一原始结果补齐 frozen bundle / LLVM prefix / oracle
  环境后由原 checker exit 0：unchecked Clang/replay `0.9998 / 0.9995`，checked
  `1.0055 / 1.0005`，proof `0.9976`，optimizer suite `1.1641`，Dijkstra
  `765667 / 350000 = 2.1876x`。report / benchmark / 最终 checker SHA-256 为
  `333a0ac36c7b1075093383efe70cf579f0acabfcf9a219ffb09789c1af87d67b` /
  `5fb86d57cc48f1815dbd38a52732a923ec865725168974343574c79740baf89b` /
  `b109162bcadbfc856d44b54e06ea954bb06a36f85e90aae08b249ddeb092b5a5`；24 个 measurement 与
  8 个 replay artifacts 均已归档。两次缺少 checker 身份环境的前置条件拒绝日志也保留，
  它们没有取新样本、修改结果或改变门槛。当前只剩新精确 SHA 十项远程矩阵。

## Task 8 行内对抗性自审

五项都由实际 Windows ARM64 原件定位：两项 host policy、一项合法 IR storage class 和一项
C oracle export 都是测试跨平台假设，不授权修改生产 ABI/ORC/export；用 policy 断言替代
cfg skip 还能保持原 92 项执行面。cache touch 则由
只读 handle 与所需 Windows access mask 直接解释，是唯一生产修复。精确属性权限比通用
写权限更小，no-follow 与 owner-only 边界不变。完整 Windows 两架构执行仍不可由本地文本
契约替代；复审未发现需要降低门槛、扩大符号面或改变缓存格式的阻断项。

## Task 9：Windows freestanding runtime 闭包与 CLI artifact 路径（I30）

精确候选 `f460c2b94f204738c2cbe6b4d9509409665a78ac` 的 run
`33349902056` 自然结束为 completed/failure：quality、native integration、Darwin
ARM64/x64、Linux ARM64/x64 与两项 performance 共 8/10 success；Windows x64 job
`99361072001` 和 Windows ARM64 job `99361071997` failure。两项 performance 保持原
schema-6、语料与门槛并全部通过：ARM64 unchecked Clang/replay `0.9997 / 0.9990`、
checked `0.9989 / 1.0012`、proof `1.0015`、optimizer suite `1.3630`；x86-64 分别为
`1.0501 / 1.0001`、`1.0192 / 0.9853`、`0.9955`、`1.5183`。因此不得重开优化器或
性能设计，也不能把八项 success 与后续 SHA 拼接。

### 9.1 复诊与不可变边界

- Windows ARM64 已通过 fact audit 7/7 和 Native 92/92，实际证明 I29 的 ABI、ownership、
  C differential export、cache warm-hit 以及 I27 RuntimeDyld 路径全部 green；CLI 21/22
  的唯一失败发生在已成功执行 `ckc build --exe` 后，fixture 仍检查未加 `.exe` 的原始 base
  path。修订只能让测试通过 `NativeArtifactPaths` 与 host platform 推导 production
  executable path；不得改 CLI 输出命名、把命令失败改成成功或用 Windows cfg skip。
- Windows x64 已通过 fact audit 7/7；Native 为 68 passed / 24 failed / 0 ignored。全部失败
  都由 in-process LLD/JIT 链接同一根因触发：`format_float.obj` 的优化代码引用
  `memcpy`/`memset`，而 `/O2 /Zl` 编译的五对象 freestanding runtime 没有定义它们，
  `kernel32.lib` 也不提供 CRT memory helpers。修订须在既有 Windows `platform.obj`
  内提供语义正确的 byte-loop `memcpy`/`memset`，并在 MSVC 下对两个定义局部关闭优化，
  防止编译器把实现重新识别为同名调用。
- runtime manifest 继续精确为 `runtime.obj`、`format_int.obj`、`format_float.obj`、
  `ryu.obj`、`platform.obj` 五个对象；不得增加第六对象、default library、动态/静态 CRT、
  新 CK public export 或 hosted runtime 依赖。`/Zl`、唯一 `kernel32.lib` allowlist、
  cache recipe/source digest、双 profile 验证、JIT/AOT 共用 runtime bytes 仍保持。
- 两个 memory helper 只补齐 C compiler 可合法生成的 freestanding 内部依赖；它们使用
  `unsigned char` 逐字节复制/填充并返回原 destination。`memcpy` 继续遵守不重叠契约，
  不擅自实现 `memmove`；长度零不得解引用。测试 source contract 必须锁定定义、局部
  MSVC optimize off/on 与五对象闭包，不能仅靠注释或 fixture 制造 green。

### 9.2 文档先行与 TDD

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改
`tests/contracts/native_toolchain.rs`、`native/runtime/windows/process.c` 与
`tests/cli/kir_inspection.rs`。

- [x] 先提交精确 run 的 8/10 终态、两项失败原件、共同根因、不变量和本计划；docs 16 与
  `git diff --check` 通过，不混入实现或测试修订。
- [x] 先给 Windows runtime production-source contract 增加 memory-helper 闭包断言，并在
  旧源码上得到真实 0/1 red；再只在既有 `platform.obj` source 增加 helper definitions，
  让同一 targeted contract 1/1 green。不得仅改测试期望。
- [x] CLI fixture 使用生产 `NativeArtifactPaths` 推导 host executable；保持原成功状态、
  产物存在性与 executable build sanitizer 断言，不按平台删除断言。

### 9.3 验证与远程闭环

- [x] targeted/default/all-feature、两种 Clippy、fmt/diff、release lib/IR/native、全部独立
  小门、artifact/compiler/JIT/version/licenses、双 prefix verifier、docs 16 与原 schema-6
  性能门全绿；测试计数、语料和阈值不降。Darwin 本地 source contract 不能替代 MSVC。
- [ ] 在实现提交的精确 SHA 上重新 dispatch 全十项 required CI。Windows x64 必须完成
  fact audit、完整 Native 92/92、CLI 22/22、compiler/artifact/JIT 与发布依赖审计；
  Windows ARM64 必须保持 Native 92/92 并把 CLI 恢复为 22/22。其他八项同 SHA 全绿。
- [ ] 同 SHA 十项全绿后才关闭 I30/I29/I28/I27/I26/I25/I23/阶段 11，并进入 01–11/99
  总验收；任何旧 SHA 的 success 都只作诊断证据。

### 9.4 实现与本地证据

- docs-first `45142fa` 后，新 production-source contract 在未定义 memory helpers 的旧
  `process.c` 上真实 0/1 red；实现后同一 contract 1/1 green，并在行内复审后进一步要求
  optimize off/on 各精确一次、两个定义必须位于边界内部、两个零长度安全 byte loop 与
  destination return 都精确存在。禁止 hosted headers、`memmove`、allocation，并继续锁定
  `/O2 /Zl` 与五对象 manifest。
- 实现提交 `991d192f13b845abc2e35e9406982093fe07b44e` 只在既有 Windows
  `platform.obj` source 增加 `memcpy`/`memset`，没有第六对象、default library、CK export
  或 ABI 变化；CLI fixture 改用 production `NativeArtifactPaths`，仍先验证 command
  success 再验证真实 host artifact 存在。pinned LLVM 22 `clang-cl` 以 ARM64 MSVC target
  和 bootstrap 同组 `/O2 /W3 /WX /GS- /Zl` 参数实际生成 COFF，`llvm-nm` 显示两个 helper
  均为本对象定义且没有同名未解析引用；真实 MSVC x64 仍由远程门签收。
- 使用 manifest SHA-256 为
  `8a0d25cdcd729cd35be139d9f3b571d3a0769a380d1fce1e9731292119dc290c` 的当前 release
  prefix 和 `b073daad34f4dfd5055614c7893b42c38f875cb54198b528729247dd3d13f934` 的 oracle
  prefix 复验：default 479、all-feature 610、Native 102、release lib 53 / IR 58、generated
  3 / mutation 10 / fact audit 7 / verifier-cache 5 / docs 16、artifact fixture 5 全部
  0 failed/ignored；两种 Clippy、fmt/diff、release compiler、artifact 与 JIT audit 全绿。
  Apple sanitizer 按冻结契约明确报告 Linux-only capability unavailable，不冒充 Linux 门。
- 首轮数值合格但使用旧 overlay 的 benchmark invocation 被 replay identity 前置条件拒绝：
  它只执行六项 optimizer 计时，未进入 Native/Clang/replay runtime 采样，未创建 measurement
  目录，也未覆盖原 `results.json`；该拒绝不记作通过。定位到旧 overlay manifest
  `b8b790dcfdd9652b1634d8d50075b1037298ec7cbcf3e7a5fefabb55d1f84874` 与冻结 bundle
  的 `8a0d25...` 不匹配后，先用正确身份对原报告做只读 checker 验证，再重新等待资格窗口。
- 新资格窗口六个 idle 样本为 `75.15 / 81.88 / 80.70 / 77.63 / 82.30 / 79.70%`，且无
  高负载编译、索引、Node、Java 或虚拟化进程；只执行一次进入完整 runtime sampling 的
  schema-6 benchmark。同一原始结果由原 checker exit 0：unchecked Clang/replay
  `1.0008 / 0.9999`、checked `1.0065 / 0.9876`、proof `1.0022`、optimizer suite
  `1.1255`、Dijkstra `727417 / 350000 = 2.0783x`。report / benchmark / checker SHA-256
  分别为 `cd2ac37789504687ba58e93edb4fea071967a42da9922ac767a98eaad714afca` /
  `b9e62f66b72c0d80850b5e5f9aac068e3334a11764a105a0216030d8373d36fb` /
  `f73cf2f122ed75275255982895034f9297caaeee522a8829cafb8028ec0ecc4c`；24+8 artifacts
  已归档，没有再计时或调整门槛。

## Task 9 行内对抗性自审

ARM64 的 command success 与 Native 92/92 把失败精确隔离到 host artifact path fixture；
x64 的 24 个失败共享同一 LLD undefined-symbol 原件和 `format_float.obj` producer，因此不把
级联数量误当成多项产品设计缺陷。在既有 platform object 内提供两个标准 memory helpers
既闭合 `/O2 /Zl` 可生成的合法依赖，又不改变五对象 manifest、CK ABI 或链接 allowlist；
MSVC 局部关闭优化只约束 helper 自身，避免自递归，不影响其他 runtime 热路径。最终仍要求
两个真实 Windows 架构及同 SHA 十项矩阵全绿，复审未发现需要降低门槛或扩大依赖面的阻断项。

## Task 10：MSVC intrinsic definition 闭包（I31）

精确候选 `991d192f13b845abc2e35e9406982093fe07b44e` 的 run `33351217336`
仍在自然运行；截至本计划提交前，quality 与 Darwin ARM64 success，Windows x64 job
`99364841264` failure，其余七项尚未结束。不得取消剩余 jobs，也不得把本轮已成功项与修复
SHA 拼接。x64 的 cold LLVM build 完成后，真实 MSVC 以既有 `/O2 /W3 /WX /GS- /Zl`
编译 `process.c`，在 `memcpy`/`memset` definitions 分别报告 `C2169: intrinsic function,
cannot be defined`；完整 job log SHA-256 为
`60112035de5a469e3d28b1bc915f4e91986a871416fc7343667dee19624db022`。因此本轮没有进入
Native suite，不能把本地 `clang-cl` COFF 生成当成 MSVC 验收。

### 10.1 复诊与不可变边界

- Microsoft 的 `C2169` 契约表明失败来自“已经被声明为 intrinsic 的函数又出现定义”；
  `/O2` 包含 intrinsic 优化，而 `memcpy`/`memset` 都具有 MSVC intrinsic form。现有
  `#pragma optimize("", off)` 只关闭 helper definitions 的优化，不能解除它们在编译器中的
  intrinsic 身份，所以 I30 的防递归措施必要但不充分。
- Microsoft 的 `#pragma function(memcpy, memset)` 在文件作用域强制这些 intrinsic 生成实际
  函数调用，并持续到源文件末尾或相反的 `#pragma intrinsic`。最小修订是在既有 optimize-off
  边界前增加一次该 pragma；它只作用于 `process.c`，源文件后续没有依赖两者 intrinsic 化的
  热路径。不得改为全 runtime `/Oi-`、移除 `/O2`、新增对象/库或回退 memory-helper 闭包。
- 五对象 manifest/order、`/O2 /Zl`、唯一 `kernel32.lib` allowlist、无默认 CRT、CK public
  exports、ABI、cache identity、CLI artifact path 与两个 byte-loop 语义保持不变。
  `#pragma function` 必须先于 `#pragma optimize("", off)` 和两个 definitions；现有 optimize
  off/on 仍精确包住 definitions，防止实现循环被识别回同名调用。

### 10.2 文档先行与 TDD

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改
`tests/contracts/native_toolchain.rs` 与 `native/runtime/windows/process.c`。

- [x] 先提交 I30 本地证据、I31 的精确 MSVC red、官方语义、修订边界与本计划；docs 16 与
  `git diff --check` 通过，不混入实现或测试修订。
- [x] 先扩展 production-source contract，要求唯一的
  `#pragma function(memcpy, memset)` 位于 optimize-off 和两个 definitions 之前；在 I30 源码
  上取得真实 0/1 red，再做最小 source 修订使同一 targeted contract 1/1 green。
- [x] 用 pinned `clang-cl` 和 bootstrap 同组参数继续生成 ARM64 MSVC-target COFF 并检查
  两个定义及无同名未解析引用；该检查只补充 source/object 证据，不替代真实 `cl.exe`。

### 10.3 验证与远程闭环

- [x] 保持原计数与阈值，重跑 targeted/default/all-feature、两种 Clippy、fmt/diff、release
  lib/IR/native、全部独立小门、artifact/compiler/JIT/version/licenses、双 prefix verifier、
  docs 16 与 schema-6 性能门。I30 的完整原始性能证据保留，不能因 Windows-only 修订而降门。
- [x] 先等待 `991d192` 的十项 jobs 全部自然终止并归档最终状态；再以 I31 实现精确 SHA
  dispatch 新的完整十项矩阵，避免同分支 concurrency 取消旧证据。
- [ ] 新 SHA 的 Windows x64 与 ARM64 都必须用真实 MSVC 完成 bootstrap、fact audit 7/7、
  Native 92/92、CLI 22/22 和 compiler/artifact/JIT/release audits；同 SHA 另外八项也必须
  success。十项全绿后才允许关闭 I31/I30 及此前阻断并进入最终总验收。

### 10.4 实现与本地非计时证据

- docs-first 提交 `53ef61e` 只包含 I30 本地证据与 I31 的 MSVC red、官方语义和修订边界。
  随后 source contract 在 I30 源码上真实 0/1 red，最小增加单个 file-local pragma 后同一
  contract 1/1 green；red / green 日志 SHA-256 分别为
  `213f52bf096dfeb7c952ea81d307cc8abf60fd1da08e1acc0abd987fb9f07d14` /
  `c8498cc84f3ca5e6ba3847c68430fbc103f43006a62f6a2671986513722474c7`。
- pinned LLVM 22 `clang-cl` 用 ARM64 MSVC target 和完整 bootstrap flags 实际生成 COFF；
  object SHA-256 为 `62d97e9ed35d52cea34fbfdc9dc9fc93a098e548145e5dd01349aeca39f63bc3`，
  `llvm-nm` 同时显示 `T memcpy`、`T memset`，且没有同名 undefined。空编译日志与 nm 日志
  SHA-256 分别为 `e3b0c442...` / `6226128e...`；真实 `cl.exe` 仍待新远程矩阵签收。
- 旧 run 的 Windows ARM64 job `99364841227` 随后也自然 failure；它同样先完成 cold LLVM
  build，再在 `process.c` 两个 definitions 上报告相同 C2169，未进入 Native suite。完整日志
  SHA-256 为 `18cea61e528bae68e562501d2dfd8592269b4f21cc12d8b2d8f1b774930c52c1`。
  两架构独立重现确认 I31 是共同 MSVC source contract，不是 x64 linker 特例。
- run `33351217336` 最终自然结束为 completed/failure、精确 8/10。success jobs 为 quality
  `99364841132`、native integration `99364841232`、Darwin x64 `99364841169` / ARM64
  `99364841260`、Linux x64 `99364841301` / ARM64 `99364841283`、performance x86-64
  `99364841101` / AArch64 `99364841106`；仅上述两个 Windows jobs failure，没有取消或
  拼接。最后结束的 Darwin x64 原始日志 SHA-256 为
  `255ac924fd7df59fc03e468de1c9957b44f7325b4cd531d525f9ca271701fcab`，其中 fact 7、
  Native 102、CLI 22 与 compiler/artifact/JIT audits 全绿；fact artifact ID `9747706336`，
  上传 ZIP SHA-256 为 `9b378f7881ce6f13d63a6aac3c99493a2fb5fb35e788b8cee51b4e5363024ff1`。
- default 479、all-feature 610（Native 102）、release lib 53 / IR 58、generated 3、
  mutation 10、fact audit 7、verifier-cache 5、docs 16、artifact fixture 5 全部 0 failed/ignored；
  两种 Clippy、fmt、release Native build、artifact/compiler/JIT/version/licenses 与 release/
  oracle 双 prefix verifier 全绿。Apple sanitizer 按冻结契约报告 Linux-only unavailable。
  初次发布审计因后续 default release tests 覆盖 `target/release/ckc` 为 feature-disabled 产物而
  fail closed；按 CI 顺序重建 Native release 并签名后从头复验通过，没有修改审计或门槛。
- 两个正式 preflight 与后续只读监视先后捕获 `0–73% idle`、多轮外部 Rust/Node/index/VM
  高负载，均未启动 benchmark；其中 preflight 原件 SHA-256 为 `679a85c6...` /
  `e789a320...`。监视严格要求六个连续 `idle >= 74%` 且高负载计数为零，曾在 4/6、5/6
  被新活动归零，没有停止外部任务或放宽规则。最终合格六样本为
  `81.73 / 85.98 / 85.58 / 85.85 / 86.23 / 79.75%`，资格日志 SHA-256 为
  `63340dd0adcefd89e217957f0fc5240c8ecfa83eebb236e271b2b9c289a723df`。
- 合格后只执行一次进入 runtime sampling 的原 schema-6 benchmark；原 checker 与归档布局
  上的只读复验均 exit 0：unchecked Clang/replay `1.0003 / 0.9979`、checked
  `1.0072 / 1.0056`、proof `1.0015`、optimizer suite `1.1068`、Dijkstra
  `726666 / 350000 = 2.0762x`。report / benchmark / checker SHA-256 分别为
  `8a1254780dc13a8cabec12e3b8849ffcc860d8c91580bf28110ed9ee1091b441` /
  `ecf718488c28849f6fea25cf695db3c7a1522e26856d383c26a8d4d875805bda` /
  `5aae632d7b04087fcfbe2dc2a1c73e561e9b19b4ca39e3b0728f913892d94839`；24+8
  artifacts 与完整 replay identity 已归档，checksum manifest SHA-256 为
  `52c4858538580c3d520cc368256087e2636c8ad0443374637be7cdf865e63d09`，没有再计时。

## Task 10 行内对抗性自审

真实 `cl.exe` 的 C2169、官方 diagnostic 与 pragma 语义构成直接因果链，不需要扩大到 LLVM、
LLD、runtime ABI 或算法设计。文件局部 `#pragma function` 比全局 `/Oi-` 更窄，且与现有
optimize-off 分别解决“允许定义”和“避免循环被重新识别”两个独立问题；两者都由 source
contract 锁定，并最终由两种真实 MSVC 架构签收。没有降低测试、性能或平台门槛，复审未发现
新的逻辑缺口。

## Task 11：COFF x64 JIT image-base 物化顺序闭包（I32）

精确候选 `5fa94b089156ecae36a24c90d4c580fc473fbd83` 的 run `33364897799` 已有八项
success；Windows x64 job `99403408409` 自然 failure，Windows ARM64 仍在 cold bootstrap，
不得取消。x64 已完成新 recipe、真实 MSVC、prefix/cache 验证及 fact audit 7/7，证明 I31
不再触发 C2169；required Native suite 为 78 passed / 14 failed。完整原始日志 SHA-256 为
`8c3a22a7d14038230d9d760d1cced0383c82e45abdb2c1b563bfaf4b08ab8b75`；fact artifact
ID `9755398106`，上传 ZIP / 原文件 SHA-256 分别为
`a95b9547cbc119b12cfe8a490af6c124c2afe7788d4fa7931d47119da17706bf` /
`61920673061079661677c0abc0e8fb8974be26c57b813d179821a52a1b7dc5b9`。

### 11.1 复诊与不可变边界

- 7 个 cache、4 个 JIT 与 3 个 run failure 都是同一 JIT 级联：COFF x64 runtime
  `.pdata` 的 `IMAGE_REL_AMD64_ADDR32NB` 被计算成 `-0xcb0` 等负值，JITLink 报
  `out of range of Pointer32 fixup`。AOT executable、differential、fact audit 与静态
  prefix tests 继续通过；不得把 14 项删减成 skip 或误诊为 cache 行为。
- pinned LLVM 22.1.8 `MapperJITLinkMemoryManager::allocate` 在首次 materialization 时把
  graph 放到新 reservation 的 `Start`，并把 `NextSegAddr..End` 保存给后续 allocations；
  后续 graph 因此向高地址增长。当前实现按顺序验证并 `addObjectFile` anchor/runtime/program，
  但 ORC materialization 仍是 lazy；随后按 `std::set` 排序 lookup 全部 symbols，MSVC
  `??_C...` runtime symbols 先于 `__ImageBase`，使 runtime 先占低地址、anchor 后占高地址。
  `.pdata` 相对更高 anchor 得到负 image-relative 值，虽在同一 512 MiB reservation 仍非法。
- 最小修复仅限 COFF x64：验证完整六个 runtime inputs 后，先把 `buffers.front()` anchor
  加入 LLJIT 并立即 `lookupLinkerMangled("__ImageBase")`，要求成功物化；之后才加入余下
  五个 runtime objects 与 program。不得靠更改全局 symbol sort、伪造绝对地址、删除
  `.pdata`、开放 process symbols、切回 RuntimeDyld、扩大 reservation 或放宽 relocation。
- x64 私有 anchor bytes/hash/cache、六对象 fail-closed 输入、五对象 public artifact、
  ARM64 RuntimeDyld responsibility、JIT W^X audit、ABI 与语言语义全部保持。I31 的 pragma
  与 memory-helper closure 也必须继续由 source contract 锁定。

### 11.2 文档先行与 TDD

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改
`tests/contracts/native_toolchain.rs` 与 `native/bridge/ckc_llvm.cpp`。

- [x] 先提交 x64 原始失败、单一根因、pinned allocator 行为、不可变边界与本计划；docs 16
  和 `git diff --check` 必须通过，不混入生产实现或测试修订。
- [x] 扩展既有 COFF JIT production-source contract，在 I32 前源码取得真实 targeted red：
  anchor `addObjectFile`、`lookupLinkerMangled("__ImageBase")`、余下对象 loop 必须按此顺序
  位于 x64 条件分支，且 lookup error fail closed；注释或单纯把 `__ImageBase` 提前排序不算。
- [x] 最小实现使同一 targeted contract green；非 COFF-x64 保持原 loop，之后运行真实 bridge
  syntax、default/all-feature 与全部局部门。source contract 只证明结构，不能替代 Windows。

### 11.3 验证与远程闭环

- [ ] 等待 `33364897799` 的 Windows ARM64 job 自然终止并归档；不重用本轮八项 success。
- [x] 保持 I31 的全部本地非计时计数、双 prefix、release/audit 与唯一合格 schema-6 性能
  报告；Windows-only materialization 修订不得重设 baseline、阈值、语料或择优重计时。
- [ ] 以实现与证据最终精确 SHA 重新 dispatch 全十项 CI。Windows x64 必须 fact 7、Native
  92、CLI 22 并完成 compiler/artifact/JIT audits，日志不再出现 C2169、negative
  image-relative 或 Pointer32；ARM64 与另外八项也必须 success。之后才允许关闭
  I32/I31 及此前阻断并进入 01–11/99 总验收。

### 11.4 实现与本地证据

- docs-first 提交 `ae564c5c82ff5eb4823036252440dfad06d7fc9f` 后，production-source
  contract 在原实现真实 0/1 red；实现提交
  `9be13325e258a2cef2789ee82853ae18b5530c37` 只给 COFF x64 增加 anchor
  `addObjectFile`、`lookupLinkerMangled("__ImageBase")`、余下对象 loop 三段有序路径，
  lookup error 继续 fail closed，非 COFF-x64 原 loop 原样保留。red / 最终 green 日志
  SHA-256 分别为 `1e1a0ef2a19223d116cd73579ec29a6ea7567872d8c7b9aa9de88e423d57706d` /
  `3894ce814560553a13e84e1ed92290fc75f3f5f0fa0c83cdf6b9b07ae97d2e72`。
- 既有两输入 bridge syntax regression 通过；另在 Apple ARM host 显式取消 ARM 预定义并
  激活 `__x86_64__`，实际编译新增分支的 syntax-only 日志为空，SHA-256
  `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`。该补充检查仍不替代
  真实 Windows MSVC/JITLink execution。
- 最终测试版本顺序重跑 default 479 / all-feature 610（Native 102）全绿，日志 SHA-256
  `25015a4a7fce1320f22a34c21d5536cca6536dd25007ffb4f08d310fe558e4b1` /
  `4fe875afddc666554855078f698cbb372d4dd40d6cfa704f2a8b232b68cc9b72`；两种 Clippy、
  fmt/diff 通过。相同生产实现还通过独立 Native 102、release lib 53 / IR 58、generated
  3 / mutation 10 / fact 7 / verifier-cache 5 / docs 16 / artifact fixture 5、release build、
  compiler/artifact/JIT audits、version/licenses、双 prefix verifier；Apple sanitizer 按冻结
  契约报告 Linux-only capability unavailable。
- Windows-only 物化时序不影响 codegen/benchmark corpus，故没有重计时或重设 baseline。
  唯一合格 schema-6 原报告仍为
  `8a1254780dc13a8cabec12e3b8849ffcc860d8c91580bf28110ed9ee1091b441`；补齐冻结 replay
  bundle 身份后原 checker 只读复验 exit 0，仍为 unchecked `1.0003 / 0.9979`、checked
  `1.0072 / 1.0056`、proof `1.0015`、optimizer `1.1068`、Dijkstra `2.0762x`。

## Task 11 行内对抗性自审

失败地址、14 项 stderr 与 pinned allocator 源码共同证明“同 reservation”不是“anchor 在
最低地址”的充分条件；补一次显式 materialization 是恢复既有设计意图，而不是引入新 JIT
策略。修复面只控制 x64 私有 support object 的 ORC 时序，不改变 allocator、权限、符号可见
范围、runtime/AOT 闭包或 ABI。最终真实 Windows 仍是行为门，未发现需要降低原验收标准的
理由。

## Task 12：Windows release audit 的 pinned inspector 闭包（I33）

精确旧 SHA `5fa94b089156ecae36a24c90d4c580fc473fbd83` 的 run
`33364897799` 已自然结束为 completed/failure、8/10。Windows ARM64 job
`99403408399` 成功完成约 5 小时 38 分的 bootstrap、fact audit 7/7、Native 92/92、CLI
22/22 与 static-CRT release compiler build，随后在 dependency audit 的第一项
`Get-Command dumpbin.exe -ErrorAction Stop` 失败；candidate 尚未被读取。完整日志 SHA-256
`77066a288bb1db4f2b97ede1baf0a397479f334138a74629080dee4b9727ac97`；fact artifact ID
`9757383387`，上传 ZIP / 原文件 SHA-256 分别为
`eab16b85762ec0f04682b5d08512e8dfee046efe5df11e75a91efe5daf017ebd` /
`541fe8d17eda3ee101c80dc820011dea30f91a23934cad5fbbe5e0647b4b546c`。release / oracle
cache ID `7155191185` / `7161426496` 均已保存，后续不得删除或绕过 cache identity。

### 12.1 复诊与不可变边界

- `scripts/audit-ckc-release.ps1` 隐式要求 Visual Studio developer environment 已把
  `dumpbin.exe` 放入 PATH；GitHub Windows ARM64 的 PowerShell job 没有该前置条件。失败发生
  在任何 `/dependents` 输出之前，不能证明 binary 含动态 CRT，也不能降低依赖审计来掩盖。
- 同一 bootstrap 已提供并验证 `CKC_LLVM_PREFIX/bin/llvm-readobj.exe`，其版本、归档闭包和
  manifest identity 已由 prefix verifier fail-closed 锁定。最小修订是从该绝对 prefix
  解析工具，要求 regular file，执行 `--coff-imports` 并检查 exit status；不得搜索 PATH、
  `vswhere` 猜目录、启动未冻结 developer shell 或回退到系统工具。
- 保持原 forbidden regex 对 LLVM/LLD/Clang/CalcKernel/libck/MSVCP/VCRUNTIME/CONCRT/
  libstdc++/libc++ 的拒绝，以及 candidate version/licenses 检查。不得新增 DLL allowlist、
  改链接参数、跳过审计，或改变 Windows runtime/bridge/Rust static CRT、artifact/ABI/cache。

### 12.2 文档先行与 TDD

**Files:** 本文、`11-release-candidate-acceptance.md`、
`../review/implementation-blockers-01.md`，随后才允许修改
`tests/contracts/release.rs` 与 `scripts/audit-ckc-release.ps1`。

- [x] 先提交 ARM64 自然终态、原始日志/artifact/cache identity、单一环境根因与不可变边界；
  docs 16 和 `git diff --check` 必须通过，不混入测试或脚本修订。
- [x] 扩展 production-source contract，要求唯一 pinned-prefix inspector、
  `--coff-imports`、missing/nonzero fail-closed 与原 dependency/version/licenses closure，且明确
  禁止 `Get-Command dumpbin.exe`；在旧脚本取得真实 targeted red。
- [x] 最小修改同一脚本使 targeted contract green；PowerShell parse、default/all-feature、
  全部局部门与原 schema-6 只读复验通过。source contract 不替代真实 Windows PE 审计。

实现提交 `bde2ed1421350d59a02034b56f7bb171b53c97e5` 的旧/新定向契约分别为 0/1 red 与
1/1 green，日志 SHA-256 `3e70a2a99f7a664512a2b1e4939d280c34ba050f0fccd81822967293a3e055d7` /
`ac9e54ea947cdac6fd4e42b817b0ab952afae0790e1339b220c5bfb81d49c8ce`；parse 空日志 SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`。最终本地 default 480、
all-feature 611、Native 102、release lib/IR 53/58 及全部阶段 11 局部门通过；schema-6 原件
`8a125478...` 只读复验通过，未重计时或改门槛。远程真实 PE 审计仍由 12.3 唯一签收。

### 12.3 远程闭环

- [ ] 以实现与证据最终精确 SHA dispatch 全十项 CI。Windows ARM64/x64 必须都从验证缓存
  完成 fact 7、Native 92、CLI 22 与 compiler/artifact/JIT audits；x64 同时证明 I32 不再有
  negative image-relative/Pointer32。另八项也必须 success，不拼接旧 run。
- [ ] 十项全绿后才允许关闭 I33/I32/I31/I30/I29/I28/I27/I26/I25/I23 与阶段 11，并进入
  01–11/99 总验收；不得为 audit 环境问题降低验收门槛。

## Task 12 行内对抗性自审

失败发生在 inspector discovery 而非 candidate inspection，且 prefix 中已有同版本、已验证、
跨 Windows 架构可用的 PE reader，因此把审计工具也纳入 pinned prefix 是收紧环境闭包，不是
放松动态依赖政策。binary、链接器与 allowlist 均不变；最终仍要求两个真实 Windows runner
执行完整审计，未发现需要扩大到构建系统或 ABI 的阻断项。

## Task 13：COFF x64 远 process call stub 闭包（I34）

### 13.1 失败证据与根因

精确 SHA `be4b77dfef6088a3707ae2725c2077f5c415d3b6` 的 run `33393261918` 为 8/10。
Windows x64 job `99491674256` 已不再出现 I32 的 `__ImageBase`/Pointer32 失败，但 Native
仍为 78 passed / 14 failed：`ckc-runtime-4.o` 对 `GetStdHandle`、`WriteFile`、`ExitProcess`
的直接 `PCRel32` 目标位于系统 DLL 高地址，随机 JIT reservation 与其相差超过 ±2 GiB。
完整日志 SHA-256 `b96aef2719f394e7ced1490695127ebdcbad2a04a23483a9c96b4482c3e5cc00`；fact artifact
ID `9758367296`。这发生在 image-base anchor 正确物化之后，不能回退 I32 或调整地址假设。

### 13.2 最小修订与 TDD

**Files:** 本文、阶段 acceptance、review，随后才允许修改
`tests/contracts/native_toolchain.rs` 与 `native/bridge/ckc_llvm.cpp`。

- [x] 先扩展 production-source contract，要求 COFF-x64-only `ObjectLinkingLayer` graph pass、
  三个精确 allowlisted external symbol、call-opcode/PCRel32 检查、R-only pointer cell、RX stub、
  `Pointer64` 闭包及 plugin 安装；旧实现取得真实 targeted red。
- [x] 使用 LLVM x86-64 JITLink 官方 pointer/stub primitives，在 post-prune、allocation 前给每个
  远 process call 建立 graph-local 间接跳板。不得改变 runtime source/hash/cache recipe、
  扩大 process symbol allowlist、搜索任意 process symbols、缩小 reservation 或开放 RWX。
- [x] contract green 后执行真实 bridge syntax、default/all-feature 与阶段 11 全部局部门；
  source contract 不替代 Windows x64 的实际 JIT 执行。

## Task 14：Windows import descriptor 精确解析闭包（I35）

### 14.1 失败证据与根因

同一 run 的 Windows ARM64 job `99491674138` 已完成 Native 92/92、CLI 22/22 与 release
build，随后在 dependency audit 失败。pinned `llvm-readobj` 的输出包含绝对
`File: C:\a\Rust_CalcKernel\...`；脚本把整份输出交给包含 `CalcKernel` 的 forbidden regex，
因此在任何 import descriptor 之前就被仓库目录名自我命中。日志 SHA-256
`48a78a2fa36f4db15ec6415fb314382d808597c157c7af1465e5880fa8f7405c`；fact artifact ID
`9758395786`。这证明 I33 的 inspector discovery 已修复，但 raw presentation 不是 dependency
集合，不能通过删掉 `CalcKernel` 或接受任意输出解决。

### 14.2 最小修订与 TDD

**Files:** 本文、阶段 acceptance、review，随后才允许修改
`tests/contracts/release.rs` 与 `scripts/audit-ckc-release.ps1`。

- [x] 行为测试用可执行 fake inspector 输出 `File: ...Rust_CalcKernel...` 与允许的
  `Import { Name: KERNEL32.dll }`，旧脚本必须真实 red；另有 forbidden name、空/畸形与 nonzero
  对照，不能只做字符串存在断言。
- [x] 解析 pinned LLVM 22 `--coff-imports` 的 regular/delay import descriptor `Name:`，要求
  至少一个合法 DLL name，并只对该集合应用原 forbidden regex；`File:`、`Symbol:`、RVA 不参与
  dependency 判定。inspector/path/version/licenses 的 fail-closed 门全部保持。
- [x] targeted green、PowerShell parse、default/all-feature 与全部局部门通过。
- [ ] 两种真实 Windows candidate 都必须完成 dependency/artifact/JIT audits，随后才允许关闭
  I33/I35。

### 14.3 实现与本地证据

- 实现提交 `592614ffd7a00ba8e77f4d8f5e63bb710b15d8e0`。I34 production-source
  contract 在旧 bridge 上 0/1 red，加入 COFF-x64-only PostPrune plugin 后 1/1 green；日志
  SHA-256 为 `40a74e9254108de4ff65396119110e7b346de96e5973735eb9ec865a247b1f45` /
  `ec9f78422ff3d332cb103555731619e83c02cf7bb3102e53412cdf78cff8237f`。显式激活
  x64 COFF 分支的 syntax-only 检查通过且日志为空。
- I35 原行为测试先证明允许 candidate 路径被整份输出扫描误拒，0/1 red；descriptor-only
  初版为 1/1 green。行内复审又发现“任意缩进 `Name:`”不等于 descriptor scope，遂增加
  scope 外 forbidden `Name:` 与“一个有效、一个缺名 descriptor”对照，初版真实 red，按
  brace depth 精确解析顶层 regular/delay descriptor 后 green。两轮 red/green 摘要分别为
  `674e8e6c...` / `18a3b2b2...` 与 `21f5386f...` / `e3c11363...`；empty、malformed、
  duplicate name、forbidden DLL 与 inspector nonzero 均 fail closed，原 forbidden regex 未改。
- 最终工作树的 default 482 / all-feature 613（Native 102）、release lib 53 / IR 58、
  generated 3、mutation 10、fact 7、verifier-cache 5、docs 18、artifact 5 全绿；两种 Clippy、
  fmt/diff、双 prefix、release build/sign、compiler/artifact/JIT audits 同样通过。Apple
  sanitizer 仅记录冻结的 Linux-only unavailable。schema-6 原报告只读复验通过，仍为
  unchecked `1.0003 / 0.9979`、checked `1.0072 / 1.0056`、proof `1.0015`、optimizer
  `1.1068`，没有重新计时或调整门槛。

## Task 13–14 行内对抗性自审

I34 只为已经允许的三项 Windows OS call 增加 JIT graph 内的范围延伸，不新增可见 symbol 或
修改 CK/runtime ABI；64-bit pointer relocation 消除的是地址范围假设，W^X 分区仍由独立
R/RX section 和原 memory audit 验证。I35 从“整份展示文本”收紧到“regular/delay descriptor
name 集合”，恰好恢复原 `/dependents` 的语义，而 forbidden 集合与所有 fail-closed 条件不变。
两项都仍以真实双 Windows job 为行为门，复审未发现需要降低既有门槛的理由。

## Task 15：Windows native artifact audit 的 pinned inspector 闭包（I36）

### 15.1 失败证据与根因

精确 SHA `6dcd2ce3af6bc4bb2c19a86ef7865811735efd58` 的 run `33397814019` 中，Windows
x64 job `99506470952` 已通过 fact audit 7/7、Native 92/92、CLI 22/22、static release build
及修复后的 release dependency audit；随后 `scripts/audit-native-artifact.ps1:31` 再次通过
runner PATH 查找 `dumpbin.exe` 并失败，尚未读取 artifact imports/exports/symbols。完整 job
日志 SHA-256 为 `9b10eec294ba922bc2f9934c64b6108bf662ba113257f70a5072807aae0f503b`。

I33 只修复了 release compiler audit；native artifact audit 保留了同类未冻结环境依赖，因此
这不是 I34/I35 产品回归，也不能由已通过的 release audit 抵消。该 run 的其他 jobs 继续自然
结束；其成功项不得与后续修复 SHA 拼接。

### 15.2 最小修订与 TDD

**Files:** 本文、阶段 acceptance、review，随后才允许修改
`tests/contracts/native_toolchain.rs` 与 `scripts/audit-native-artifact.ps1`。

- [ ] 先增加可执行 fake inspector 行为测试：允许 artifact 通过；program 非唯一
  `kernel32.dll` dependency、module import、缺少 `answer`/forbidden export、runtime forbidden
  symbol 与 inspector nonzero 分别失败。在旧脚本真实 red，并由 production-source contract
  禁止 `Get-Command dumpbin.exe`。
- [ ] 只使用同一已验证 `CKC_LLVM_PREFIX/bin/llvm-readobj.exe`：`--coff-imports` 解析 program /
  module 顶层 regular/delay descriptor，`--coff-exports` 解析 module 顶层 export，`--symbols`
  检查 runtime objects。缺 prefix/tool、任一 inspector nonzero、畸形 scope/name 均 fail closed。
- [ ] 保持 program dependency 必须精确为唯一 `kernel32.dll`、module 无 imports、必须导出
  `answer` 且拒绝 LLVM/LLD/Clang/CalcKernel/`__ck_`、runtime memory/locale symbols 禁止及
  SHA256SUMS 验证；不得改 artifact、linker、ABI、cache、allowlist 或 CI required 状态。
- [ ] targeted green、PowerShell parse 与完整本地门通过；最终双 Windows job 均完成 release /
  artifact/JIT 三项 audit，同一新 SHA 十项全绿后才允许关闭 I36 及此前远程项。

## Task 15 行内对抗性自审

已验证 prefix 本来就安装、散列并校验 `llvm-readobj.exe`，它能覆盖 COFF imports、exports 与
symbol table；因此修订只关闭第二个 audit 脚本的 PATH 缺口，不引入新工具或改变产物政策。
真实 artifact 内容尚未被失败 job 审计，所以本地 fake inspector 只能锁定 parser 与
fail-closed 边界，最终仍必须由两个 Windows candidate 执行签收。当前未发现需要扩大到
bootstrap、runtime 或链接参数的设计阻断。
