# CK 0.11 阶段 11：首轮候选 CI 阻断复诊

## 范围与证据

本复诊只处理候选实现进入真实六 host 后暴露的可复现环境/实现缺陷，不重开已经通过的
语言与优化器设计，也不改变性能、正确性或六 host 全绿门槛。原始证据保留在候选 CI
run `33258768178`（commit `d8d7f903bed9a215e78986634d1f2c29cc264bee`）。

## I01：Windows bootstrap 未把 CMake 绑定到 MSVC

- x86-64 Windows job `99116972015` 与 ARM64 job `99116972007` 的 CMake 探测分别选择
  GNU 15.2.0/14.2.0，生成 `.a`，随后都被既有 `LLVMDTLTO.lib` 完整性门拒绝。
- 这不是缺库或可放宽的后缀差异：release target 是 `*-pc-windows-msvc`，接受 GNU archive
  会使 compiler/CRT/ABI identity 与六 target 契约不一致。
- 修复在 `scripts/bootstrap-llvm.ps1` 显式传入 `cl.exe` 作为 C/C++ compiler，并保留 `.lib`
  完整性检查。后续内联复审又确认仅写 compiler name 仍可能因 shell 未加载 MSVC 而失败，
  因此脚本必须经 `vswhere`/`VsDevCmd` 导入环境、使用受支持的 amd64 host tools，并以
  `_M_ARM64`/`_M_X64` 预处理探针和 CMake compiler-ID 双重验证目标/identity；不得改成同时
  接受 GNU archive。首个契约测试先以缺少 compiler 参数失败并在 commit `8017518` 通过，
  第二个契约测试再以缺少 MSVC environment 导入失败后驱动完整修复。

## I02：Darwin 把“无线程级 MAP_JIT”误判成“无安全 JIT”

- macOS ARM64 job `99116972016` 运行在 `macos-15-arm64` image 15.7.7；pre-LLVM fact audit
  已通过，随后 Native suite 因 `Darwin JIT thread write protection is unavailable` 出现
  16 个级联失败，结果为 75 passed / 16 failed。
- Apple SDK 的 `pthread_jit_write_protect_supported_np(3)` 契约是查询 `MAP_JIT` 区域的
  **per-thread** write protection。返回 false 只否定该机制，并不否定用普通 RW/NX 映射加
  页级 `mprotect` 完成 `RW -> RX/R-NX`。本机 arm64/x86_64 双架构探针也分别复现返回
  true/false。
- 原实现无条件要求 true，因此既错误拒绝受限 ARM virtual runner，也必然拒绝 Darwin
  x86-64。安全修订是两条互斥路径：能力为 true 时使用 `MAP_JIT` 与线程切换；能力为 false
  时使用普通 RW/NX reservation，并在 materialization 后逐 segment 页级 finalization。
  两条路径都必须由同一 audit 证明 relocation=RW/NX、code=RX、data=NX 与 instruction-cache
  finalization；不允许 RWX fallback。
- 最后完成的 macOS x86-64 job `99116972003` 也先通过 bootstrap 与 pre-LLVM fact audit，
  随后 full Native suite 在 cache/run/sanitizer/executable 的 JIT 消费路径出现成组失败并最终
  SIGBUS。它没有暴露新的 bootstrap、fact audit 或 ABI 阻断，仍由同一 Darwin capability/
  finalization 修订覆盖；修复后的 x86-64 job 必须以完整 suite 和 memory audit 证明这一点。
- Repository contract test 先以缺少双路径失败。实现后，本机真实 MAP_JIT 路径的 JIT tests
  为 5/5；临时强制关闭能力探测后完整 Native suite 为 91/91，随后立即恢复真实探测。正式
  shell audit 仍以 hardened runtime 和唯一 `com.apple.security.cs.allow-jit` entitlement
  运行，并拒绝不一致的 capability tuple。

## I03：可选 TypeScript oracle 环境泄漏到 Native jobs

- Linux ARM64 job `99116972014` 的 Native suite 为 91/91；随后 CLI suite 因 workflow
  全局设置 `CALCKERNEL_TS_ROOT`、但该 job 并未 checkout 对应目录而得到 20/21。
- Readiness test 的正确契约是“环境变量存在就严格验证，未配置就不宣称拥有 oracle”；
  quality job 已固定 checkout/build 精确 oracle commit，Native/release jobs 则按发布契约
  自包含且不依赖该可选仓库。
- 修复把变量从 workflow 全局移到唯一 owning quality job。仓库契约测试先复现变量泄漏，
  再要求全 workflow 只出现一次、header 不得配置、quality slice 必须配置精确路径。没有
  删除或 ignore readiness test；quality 继续实际执行正向验证，其他 jobs 继续执行缺失配置
  的负向/可移植性边界。

## I04：绝对 V0.10 时延错误假定 hosted performance worker 受控

- AArch64 performance job `99116971909` 唯一门禁错误为
  `unchecked/proof_loop regressed more than 8% from pinned v0.10`。artifact
  `performance-AArch64`（ID `9718052719`，zip digest
  `9941dec924ac90b89d8a121f651c603077f790443a40682640e6350d5f14d4d8`）记录候选
  Native/Clang 分别为 8,003,333/8,004,085 ns；固定 V0.10 run `33250945293` 的同项
  Native/Clang 则为 7,300,126/7,275,959 ns。两次运行内 CK 都与同机 Clang 等速，只有
  跨 hosted runner 的绝对时延共同移动约 10%，因此失败不是 0.11 codegen 回退。
- 原规范写“controlled workers”，实际 workflow 使用 GitHub-hosted x86-64/AArch64 pool，
  没有物理机器或频率 identity。这是比较模型的真实错误，不能靠重跑或把 8% 改大解决。
- 首个 TDD 修订把 runtime report 升级为 schema 5，并要求每项同时携带冻结
  `v010MedianNs`/`v010ClangMedianNs`；门禁保留 3%/8% 数值不变，改比较
  `(T0.11-Native/Tcurrent-Clang)/(T0.10-Native/T0.10-Clang)`。Synthetic common-mode
  用例先红于旧 schema/checker，再在新 checker 下通过；无效 V0.10 Clang oracle 必须失败。
- 对该修订内联复审时又发现：若 `Tcurrent-Clang` 编译候选 0.11 自己导出的 C，KIR 层同时
  影响 Native/C 的退化可能被抵消。最终模型因此进一步收紧：精确 V0.10 compiler 通过与
  固定基线 harness 相同的 `check -> lower_to_mir -> emit_c_module` 路径生成 checked/
  unchecked 共八份 C oracle；它们的 SHA-256 纳入 baseline identity，0.11 harness 只编译
  这些冻结 source。这样配对分母只吸收 runner 共模变化，不会掩盖候选 frontend、KIR 或
  Native 退化。
- 再次复审还发现仅要求配对 median 为正数不足以绑定 manifest；故障注入把合法正数 100
  改成 99 后旧 checker 曾错误接受。最终 checker 会读取 schema-2 manifest，并按
  target/CPU/mode/case 精确比较两个报告字段；该合法篡改用例现在稳定失败。
- 三架构 V0.10 配对 median 均来自固定捕获：Linux AArch64/x86-64 使用 run
  `33250945293` 的两个成功 artifact，本地 macOS AArch64 使用精确 V0.10 worktree 原始
  schema-2 report。所有 24 个 runtime 条目都必须有 Clang median，八份冻结 C source 都
  必须通过摘要契约；不得改动语料或阈值来迁就候选。

## I05：KIR O3 在较大控制流语料上超过 individual 3x 时延上限

- x86-64 performance job `99116972035` 的 artifact `performance-x86-64`（ID
  `9718529599`，zip digest
  `95a7226f244674ca83b8658f0e7b90790a0051257d66ec24fa0bff5ae3332df8`）记录
  `example-dijkstra` 的 KIR/V0.10 MIR optimizer median 为 3,648,012/832,254 ns，
  即 4.38x；AArch64 对应项为 3,148,172/703,886 ns，即 4.47x。runtime 的 Native/Clang
  语义与吞吐门都未指向同一退化，因此这是 compiler latency 的独立真实阻断，不能归因于
  hosted runner 共模漂移，也不能提高 3x 门槛。
- release 阶段计时把热点定位为两部分：unchecked KIR 中没有 `Guard`，但两轮
  `sccp-range` 仍对所有函数构造完整 product-domain result，随后没有任何消费者；每次
  changed-pass 验证成功又为验证缓存深拷贝完整 KIR。前者是缺少 demand boundary，后者把
  debug 防御性核验成本带进了 release 热路径。
- TDD 先增加两条行为契约：unchecked/guard-free Dijkstra O3 必须记录 0 个 scalar analyzed
  function；checked guard case 必须继续记录至少 1 个。实现让 `sccp-range` pass、顺序与
  verifier record 保持不变，只对含 guard、可能参与 check elimination 的函数执行 scalar
  analysis。所有实际 rewrite 后仍运行完整 KIR/fact/proof verifier。
- no-change verification cache 在 debug 构建继续保存并逐字段比较 KIR/proof/elimination/
  contract 快照，以捕获错误的 pass change declaration；release 只复用该内部 change
  contract，避免深拷贝。结构验证器只把不参与迭代输出的 lookup/set 从树结构换为预分配
  hash 结构，错误遍历顺序和诊断保持由 module 顺序决定。输入失败仍返回原 module，artifact
  transaction 边界未改变。
- 修复后 debug/release optimizer suite 均为 49/49，全特性测试和 all-feature Clippy 通过。
  高负载本机的非规范 quick 复诊中六项 KIR/V0.10 比率分别为 0.85x、0.63x、0.56x、
  0.71x、0.53x、1.78x；`example-dijkstra` 为 923,917/518,666 ns。该结果只证明修复数量级，
  最终判定仍必须由修复 commit 上的 x86-64/AArch64 完整 performance jobs 给出。后续 run
  `33267646660` 的 AArch64 performance job `99140467105` 已正式通过：unchecked/checked
  Clang mean 为 1.0002/0.9995，配对 V0.10 ratio 为 0.9990/0.9992，proof-loop ratio 为
  1.0253，optimizer suite-median ratio 为 0.7880。x86-64 job `99140467190` 也已通过，
  对应数值为 1.0499/1.0047、1.0006/1.0118、0.9954 与 0.8659；最终修复 commit 仍须
  重新给出同提交证据。

## I06：Windows release prefix 完成后重复创建 manifest 目录

- 修复候选 run `33267646660` 的 Windows x64 job `99140467224` 与 ARM64 job
  `99140467113` 都已成功使用 MSVC 完成
  LLVM/LLD release prefix 安装，并继续通过 `LLVMDTLTO.lib`、runtime C object 与
  `kernel32.lib` 生成检查；随后在脚本第 224 行执行
  `New-Item -ItemType Directory -Path $manifestDir` 时失败，因为更早的 runtime 步骤已用
  `-Force` 创建 `share/ckc/runtime`，父目录 `share/ckc` 必然已存在。
- 这是 bootstrap 尾部的目录幂等性错误，不是编译器、MSVC identity 或 archive 格式错误。
  契约测试先要求 manifest directory creation 可容忍 runtime 先创建父目录并稳定失败；实现
  只给该 `New-Item` 增加 `-Force`。修复后针对性 Rust 契约测试与 PowerShell AST parse 均
  通过；不得通过删除 manifest/evidence 写入来绕过。

## I07：ELF 审计把非加载的 LLD provenance 误判成运行时依赖

- 同一 run 的 Linux ARM64 job `99140467324` 已通过 bootstrap、pre-LLVM fact audit、Native
  91/91、CLI 21/21、release build 与 compiler dependency audit，唯一失败是 generated
  artifact audit 报 `forbidden ELF producer marker`。该 audit 同时已经证明 dynamic/executable
  无 `NEEDED`、static executable 无 undefined symbol、dynamic export 精确为 `answer`。
- LLVM 22.1.8 的 ELF LLD 对每个非 relocatable 输出无条件生成非 `ALLOC` 的 `.comment`，固定
  内容为 `Linker: LLD 22.1.8`；这说明构建来源，不会被 loader 映射，也不是运行时依赖。
  旧脚本却复用了 dependency 名称正则扫描 `.comment`，所以任何合规的 embedded-LLD ELF
  dynamic/executable 都必然被自己拒绝。
- TDD 用 mock GNU binutils 构造零依赖 ELF 审计面，旧脚本先稳定失败。修订保留全部 dynamic、
  undefined-symbol、export 与 runtime-object 门，并进一步要求 linked ELF 的 `.comment` 必须
  非 `ALLOC` 且包含精确 pinned LLD marker；不得简单删除 producer provenance 检查或接受任意
  LLD 版本。

## I08：Darwin entitlement audit 依赖非稳定的人类展示格式

- 同一 run 的 macOS ARM64 job `99140467216` 已通过 Native 91/91、CLI 21/21、release compiler
  dependency audit 与 generated artifact audit；仅 JIT shell audit 在 hardened ad-hoc signing
  后报 `hardened candidate has unexpected entitlements`。runner 是 macOS 15.7.7；同一脚本和
  policy 在本机 macOS 26.6.2 完整通过，说明不是额外 entitlement 或 JIT 权限本身失败。
- 原脚本以旧式 `codesign -d --entitlements :-` 取得 abstract representation，再用
  `plutil -p` 的人类可读行数和字符串拼写判定唯一 key。该展示不是跨 macOS 版本的机器契约；
  `codesign` 的 documented extraction surface 是显式 `--entitlements - --xml`。
- 契约测试先要求 documented XML extraction、canonical plist conversion 与 exact comparison
  并对旧脚本失败。修订将实际 entitlement 与仓库 policy 都规范化为 binary plist 后用
  `cmp` 比较，因此仍要求且只接受唯一 `com.apple.security.cs.allow-jit=true`，没有增加
  `disable-library-validation`、`unsigned-executable-memory` 或其他豁免。本机真实 hardened
  JIT audit 在修订后通过；macOS 15 仍须由下一轮同提交 job 复验。

## I09：Darwin x86-64 SIGBUS 的 entry 假设已被否定，阻断仍未解决

- Run `33267646660` 最后完成的 macOS x86-64 job `99140467067` 运行于 macOS 15.7.9；
  bootstrap、pre-LLVM fact audit 及其 artifact 均成功。I02 修订后，首轮曾失败的全部 cache/run
  cases 已转绿，但完整 Native suite 仍在需要执行 standalone Mach-O 的 scalar sanitizer cases
  出现失败，测试进程最终以 SIGBUS 终止；这不是 I08 entitlement audit，因为失败发生在
  release build/audit 之前。
- `6892182` 曾根据旧 `ld(1)` 的 CRT entry 说明提出 stack-entry 假设，并增加
  `__ck_start` wrapper；本机 91/91 与 Rosetta 均通过，但这些证据不能证明假设成立。
- Run `33277614781`、同 SHA 的 Intel job `99167002488` 再次在同两项 scalar sanitizer
  test 失败，Native test 进程仍以 SIGBUS 退出。Bootstrap、cache save、fact audit 7/7
  成功，因此 entry wrapper 没有解决阻断，不能列为已确认修复。
- 进一步核对 [Apple dyld4 源码](https://github.com/apple-oss-distributions/dyld/blob/main/dyld/dyldMain.cpp)：
  `LC_MAIN` 被转换为 `MainFunc` 并以普通 C ABI 调用，返回后由 libSystem exit；只有旧
  `LC_UNIXTHREAD` 分支走 `gotoAppStart`。先前把现代 `LC_MAIN` 等同于 raw kernel stack
  entry 的解释错误，现明确撤回。Wrapper 的必要性、相关注释/契约必须在真实 crash
  backtrace 确认根因后重新审查，不能把仅断言 wrapper 存在的测试当作 bug regression。

## I11：Native 进程崩溃使 libtest 缓冲中的失败详情丢失

- 上述 Intel log 只有两个 `FAILED` 名称和父进程 SIGBUS；没有 assertion detail 或 native
  backtrace，无法区分 standalone child failure 与随后父进程中的 JIT/dylib/LLVM failure。
- 先增加两个失败的 CI contract tests，再保留原并行 required suite，只增加 `--nocapture`
  与 `RUST_BACKTRACE=1`。仅该 step 失败且 host 为 Darwin 时，额外用 LLDB 串行/并行回放
  同一完整 suite，保存 crash stack、命令结果与匹配的 DiagnosticReports。诊断可失败，
  但原 required step 失败永远保留，诊断结果不能替代验收结果。
- Runtime/LLVM 输入在此诊断轮保持 `6892182` 不变，复用精确 identity 的已验证 prefix，
  以便先收集原故障的真实 stack；不是通过 cache 忽略新 runtime source。

## I10：LLVM prefix cache 未绑定 runtime 输入且只在整 job 成功后保存

- I09 的试验重编了 Darwin `platform.o`，但旧 cache digest 只包含 LLVM manifest 与两个 bootstrap
  script；`native/runtime/**` 完全不参与 identity。命中旧 prefix 会让新 compiler 静默嵌入旧
  runtime object，manifest 内 object hash 仍自洽，现有 validation 无法知道 repository source
  已变化。这是供应链正确性缺口，不能靠再改一次 bootstrap script 人工撞 key。
- 同一 macOS x86-64 job 已成功完成约三小时 bootstrap，但随后 Native 失败；旧
  `actions/cache` post 没有保存 Darwin prefix，仓库 cache list 也没有对应 Darwin key。若每次
  downstream 失败都丢弃已验证 prefix，诊断循环会重复冷构建且无法稳定复现。
- TDD 先要求 runtime C/header/assembly/definition/text-stub 输入进入 `hashFiles`，并要求 pinned
  `actions/cache/restore`/`save` 在 validation 前后分别恢复、保存 release/oracle prefix；旧 action
  稳定失败。修订保留原 manifest/object/version/static validation 与 cache key 内容寻址，只把真实
  输入纳入 digest，并在 prefix 自身验证成功后立即保存；restore-only action 不会在 job 结束时
  重复 post-save。下游测试仍是 required gate，缓存成功不等于 candidate 成功，也不降低任何验收门槛。

## I12：Darwin 复用 JIT 默认 Large code model 产生只读 text fixup

- 诊断 run `33287902589`、SHA `c1e1322cb0e1aa67f6cf8ff6e381fcec05ce87da`、Intel
  job `99194245341` 的 serial/parallel LLDB 都在 dyld `applyFixupsGeneric` 的写指令
  停住；serial 调用链为 differential O0 library 的 `dlopen`。Standalone sanitizer 的
  DiagnosticReports 同样记录 `KERN_PROTECTION_FAILURE`，目标位于 RX `__TEXT`，在
  dyld prepare 阶段、用户 `main` 执行之前。它不是 C ABI stack、sanitizer predicate
  或并发调度问题。
- `lldb-serial.log` SHA-256 `45d1677a0e1b1261d5868e8146df357622a094226c60a193fde7c627422b551e`；
  parallel `b9aed9a6a8dee047a8cc1b80b943c30e9fdee708120d6e5e4b5f29e905f868ba`；
  一个 standalone crash report `6421421c4e9f0e78f4add8e999b9f3a239c56f3d423739d0b7ecc5b846e9850e`。
- 固定 LLVM `22.1.8` 的 `JITTargetMachineBuilder.cpp` 调用 target constructor 时传
  `JIT=true`；`X86TargetMachine.cpp::getEffectiveX86CodeModel` 因此默认 Large；
  `X86Subtarget.cpp` 明确只有 ELF 有真正的 Large PIC，其他格式使用绝对引用。CK 仅
  设置 PIC，遗漏 code model，却把同一 object 用于 Mach-O AOT 与 ORC。
- 同一 scalar sanitizer IR 用固定 X86 `llc` 对照：Large+PIC 的 `__TEXT,__text` 有三条
  `UNSIGNED / pcrel=False / quad` relocation，Small+PIC 则全部是
  `BRANCH / pcrel=True / long`。这解释 Intel dyld 的写保护失败，也说明此前 Rosetta
  以及默认 Small 的离线 `llc` 不能复现该错误配置。
- 先以 repository contract 观察缺少显式 Small 的预期失败，再给 Mach-O target 设置
  Small；其他平台保持原 code model。补充真实 Mach-O object test，在 O0 保留 internal
  call 并检查 executable `__text` relocation，禁止 absolute pointer fixup；既有完整
  O0–O3 differential、standalone、sanitizer、ORC tests 仍必须通过。
- I09 的 wrapper 没有必要性证据，待本次单变量 code-model 修订在 Intel 验证后，撤销
  wrapper 及错误的 entry 注释/规范。最终不得保留“LC_MAIN 是 raw kernel entry”的
  说法，也不得通过修改段为 RWX 或放宽 dyld/audit 解决故障。
- 同 SHA 的 Intel 重跑 job `99196059470` 已实际通过 fact audit 7/7、完整 Native
  92/92（含原 SIGBUS 的两类 sanitizer 与 O0 differential）、CLI 21/21；Small 修订的
  单变量执行证据成立。I09 wrapper 与错误规范已在 `f43a5f4` 撤销，完整矩阵仍待验证。
  该 job 的后续独立签名审计失败见 I15，不能把整个 job 记为成功。
- 诊断 run 已保留 7 个成功 job 与 Intel crash artifact；收集完成后取消该 run 中重复
  的 Windows 冷构建，原 run `33277614781` 的两个 Windows 构建继续，不把被取消 job
  计作验收通过。

## I13：Windows checkout 的 CRLF 转换破坏 provenance 字节

- Run `33277614781` 的 Windows x64 job `99167002466` 已完成 MSVC LLVM/Clang
  bootstrap 并保存 release/oracle cache，但在首次 Cargo build 的 provenance 检查失败。
  `RUST-COPYRIGHT` 期望 SHA-256 为
  `172020dbfd5b53a226dfde77616190a48dcff519b0bc0e6deb91a8450782c4af`，实际为
  `dddcd10d99c349f384aa10b9d536239c577005328fcf317c19d8a1291a3385b9`。
- 本机执行 `git -c core.autocrlf=true cat-file --filters HEAD:third_party/licenses/RUST-COPYRIGHT`
  精确得到同一错误哈希，证明是缺少 repository checkout policy，而非下载篡改或 expected
  checksum 错误。`Cargo.lock` 的多行 identity、snapshot 与 runtime source hash 同样需要
  稳定字节，不能只特判这一个 license。
- 先加真实 Git filter regression 并观察预期失败，再增加 `.gitattributes`：仓库文本统一
  LF，vendor license/source 禁止 newline conversion、保留原始字节。不修改任何 pinned
  checksum，不在 verifier 内 normalize，也不设置 user/global Git configuration。
- 已检查当前 index 没有 CRLF/mixed text blob；无需重写源文件。属性本身会使 Windows
  runtime recipe hash 与 Unix 使用同一 canonical 输入；旧 CRLF cache 不得作为新输入的
  完整 prefix 命中。
- 本机上述 Git filter regression 从失败转为通过；随后 Rust 1.90.0 的 fmt、all-feature
  Clippy、默认/全特性完整 tests、release build、compiler/artifact/JIT audit 全部 exit 0。
  这只构成本机证据，Windows runner 仍须实际验收。

## I14：x86 跨 runner 配对时延出现非共模变化，待同机复诊

- Run `33288190232`、SHA `4a2969d`、performance x86 job `99195017517` 被原门禁
  拒绝：unchecked `integer_accumulate` 的归一化 V0.10 ratio 超过 1.08。原始 artifact
  ID `9725146781`、zip SHA-256
  `ce84607e4f27dfe4b162be17f180c70ccca8b2c17b6b05322128991f0a10d071` 已保存。
- 本轮 Native/Clang median 为 `24770394/24764265` ns，固定 V0.10 为
  `23000767/27975716` ns；此前通过的候选 `6892182` 为 `23008527/27983810` ns。
  两轮候选间 Linux 生效的编译路径、语料、Clang 参数没有变化，唯一 target-machine
  变更受 Mach-O 条件限制。但缺少 CPU identity 和本轮同 worker V0.10 结果，尚不能
  排除环境差异或声明为代码修复，尤其不能只重跑直到通过。
- 先新增失败 CI contract，再添加原 gate 失败后的同 worker 诊断：记录 `lscpu`，
  checkout 精确 V0.10 commit，只应用四份已有摘要固定 adapters，复跑完整 V0.10
  runtime/optimizer harness，保留原始结果及两版 integer kernel executable `.text`。
  原 gate、冻结数值、3%/8% 与其他阈值完全不变；诊断不能覆盖 required job 失败。
  本轮使用默认关闭的显式 dispatch input 强制采集同样的诊断，以免把“下一轮偶然通过”
  当作解释；后续常规运行仍在失败时自动采集。
- Intel job `99195017518` 同轮另因 GitHub artifact CreateArtifact 连续五次 timeout
  在完整 Native suite 之前失败；保存原日志后仅重跑该 job 的同一 SHA，用于 I12 单变量
  验证。上传故障不构成 Small code model 修复成功的证据。

## I15：Intel Cargo 产物未自动签名，CI/release 缺少显式签名步骤

- Intel job `99196059470` 的 Native/CLI/release build 全部通过，随后
  `audit-ckc-release.sh` 原样拒绝 `code object is not signed at all`。完整 log SHA-256
  `1fa5b20cc68377f13cad2aac89663186c6b2bbdc91c554e02c501adfdcc6da14`。现有 macOS
  ARM linker 自动 ad-hoc 签名不能被当作所有 Darwin host 的契约。
- 检查发现 CI 与 release artifact workflow 都没有显式签名实际 compiler；JIT audit
  只签名临时副本，不能满足前面的严格 compiler signature audit，也不能保证打包原件。
- 先增加覆盖两个 workflow 的顺序回归并观察失败；本机移除一个临时 compiler 副本的
  签名后，原 audit 稳定以同一 `not signed` 错误退出 1。修订在两个 workflow 的正确
  位置添加现有 policy 的 `codesign --force --sign - --options runtime --entitlements`，
  目标是实际 compiler。未改 audit，未添加 entitlement，未引入证书或 notarization。
- 对同一 unsigned 副本使用该签名命令后，compiler audit 与 hardened JIT audit 都 exit 0；
  两 workflow 的 source-order regression 转绿。Intel hardened JIT 仍必须在修订后的
  完整 CI 中实际验证，不以本机结果替代。

## I16：prefix 保存前的验证尚未执行对象哈希，且 release 保存晚于 oracle build

- 对 I10 修订自审发现，action 的 `Assert-Prefix` 仅匹配 manifest 子字符串并查询
  `llvm-config --version`；真正的 runtime object/import hash validation 在后续 Cargo
  build 才运行。因此“manifest/object validation 后保存”的计划约束尚未由 cache
  boundary 实现。release prefix 的保存还排在完整 oracle build 后，后者失败时会丢失
  前者已经完成的冷构建。
- 两个新契约先在旧实现失败。修订独立 PowerShell verifier，在缓存恢复/保存边界验证
  唯一且大小写准确的 schema/target/profile/version/source/static 字段、所有声明的 static
  library、恰好五个规范 runtime object 名称及 SHA-256、Windows import 名称及 SHA-256，
  并拒绝 shared LLVM 与错误的 Clang profile/version。不改现有 checksum 或 cache identity。
- release prefix 在独立验证成功后立即保存，再构建 oracle；oracle 也独立验证后保存。
  未通过验证的 prefix 不得被保存或暴露给 Cargo，原后续 Cargo/Native/audit gate 继续保留。
- 本机 mock Unix/COFF prefix 正例通过；runtime/import corruption、路径穿越、重复 key、
  大小写错误 key 与伪装 static flag 均被拒绝。真实临时 release overlay 也经过同一 verifier；
  补记其已经存在的 LLVMDTLTO archive 到 overlay manifest，没有改动 baseline worktree/prefix。
  测试执行使用本机已有 PowerShell 7；双语入门文档声明该测试工具依赖，不改变发布产物依赖。
- 本机 Rust 1.90.0 fmt、all-feature Clippy、default/all-feature tests、release build 与三类
  compiler/artifact/JIT audit 全部 exit 0。完整 default/all-feature logs 的 SHA-256 分别为
  `50b3ab1222c2891acac57e480251efbb4a880b4738a4cc7711366e81f30d30dd`、
  `4079c00dcd650b8c392f5c2c237966651308f7971b39f1a1d52428166cbe5e81`。
  验证后的临时 release overlay manifest SHA-256 为
  `b8b790dcfdd9652b1634d8d50075b1037298ec7cbcf3e7a5fefabb55d1f84874`。

## 修订边界（全部阻断，持续有效）

- 同步修订 Native LLVM ABI 与 release 双语文档、阶段 11 task/acceptance 和仓库契约测试。
- 不跳过任何 Native/JIT/cache/run test，不把失败 job 改成 optional，不降低性能门槛。
- 本轮修订必须在同一 commit 上重新通过 quality、native integration、六 native host 与两
  performance runner；在此之前不能把各轮本地或部分 host 成功汇总为远程验收完成。

## 修订后对抗性复审

待修复后的全量本地与六 host 证据完成后追加。复审重点是 Windows archive/CRT identity、
Darwin 两条路径的 W^X 互斥性、audit 是否可能接受不一致 tuple、TypeScript oracle 配置
是否仍跨 job 泄漏、performance 分母是否确为摘要固定的 V0.10 C source、release
no-change cache 是否只复用准确的 pass change declaration、guard-free demand skip 是否会
漏掉安全消费者、Darwin object 是否没有 absolute text fixup、dyld C-ABI entry/exit 是否
正确、runtime cache 是否可能命中旧 source、Windows checkout 是否保持 provenance
字节，以及是否有测试被跳过。
