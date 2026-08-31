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
  **复诊撤销：release 只信任 change declaration 的部分违反冻结的 proof-preservation
  约束，已在 I17 通过故障注入证实并撤销；I05 的其余性能修复不受此结论影响。**
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

### I14 同 worker 诊断结果（2026-08-30，门禁尚未关闭）

- Run `33288920505` / SHA `c91c2c08b6ffacb3ae77835921b5c5febd396ed9` 的 x86 job
  `99196977267` 再次以相同 unchecked/integer gate 失败。CPU 为 Intel Xeon Platinum
  8573C（family 6/model 207）；候选 Native/Clang 为 `14671625 / 14671807 ns`，同 worker
  精确 V0.10 为 `14665704 / 14666416 ns`。两版在该 worker 上均与 Clang 等速，而冻结
  跨 worker 配对仍是 `23000767 / 27975716 ns`，即连未改动的 V0.10 自身也会被这项
  跨 worker 比率判为回退。这构成 I04 “Clang 归一化足以消除不同 worker 影响”的反例，
  后续须在不降低 3%/8% 门槛、不改变固定编译器身份的前提下复诊比较协议。
- 诊断脚本导出的 x86 `.text` 是零字节，因为实际代码位于 `.ltext`；其空文件相等不构成
  机器码证据。另行比对上传的完整 ELF，candidate 与 V0.10 在每个模式下逐字节一致：
  unchecked SHA-256 `89ecf8a14d53e21fec95c57cba926a282e1100f123752cda7f193f3eef609510`，
  checked `82183fcb2f5eb9118fe98dac90795cfaaa8670ad465eaa12001fde13b327d0cc`。
  该结果不替代原失败，也不自动构成新协议下的验收通过。
- Artifact `9726612505` 的 zip 摘要为
  `f7a6c096aeab13f712373b158d0013f31e4992754d25a0191c8e0ab49199cd08`；候选与同机 V0.10
  report 摘要分别为 `ce410c125464037dabda7429cb42e22108d0d1ac926862ea9e5025f5b5c4d0a0`、
  `e668ec15347b73ca0b7b8b21b11161c3cb557f21de82a218faad79f00ad75285`。完整 job log 摘要
  `5cf9cc10eb0f3c6f7d488d669c26952486c498219d8c4bdefa40c5ad6a1c92ae`。

### I14/I19 比较协议复诊与修订（尚待实现/验收）

- 上述同 worker V0.10 自比较反例足以否定“历史 Native/Clang 比率可以跨任意 worker
  复用”的假设；它不说明源码或数值门槛需要改变。修订为独立构建同一固定 V0.10
  编译器，把其真实产物与当前 Native、两份同源冻结 Clang oracle 在同一进程重放。
  四项公式保持，geometric 3%/individual 8% 仍用原阈值，历史 manifest 数字完全不改。
- I19 的多个 Native/Clang 与固定 V0.10 报告都受跨模式整套运行时段分隔影响；改为
  双版本/双模式八通道轮转，固定三轮 warm-up、二十样本、每样本 min-of-seven 与
  两千万 batch。原始 checked/unchecked 97% 门槛不做校准或归一化。此修订不声称
  具体频率/调度原因已经确定，也不声称新协议必然通过。
- 双语规范/guide、schema 6、master、阶段 11 与总验收同步；完整执行计划为
  `../implementation/11-runtime-replay-plan.md`。独立 baseline clone、四个固定 adapter、
  实际 compiler/library 摘要、source/recipe identity、精确输入/顺序、负例门禁和保留
  原始报告形成闭环。自审未发现范围或证据缺口，先提交文档，再 TDD 实现。
- 未改动任何 frozen CK/C source、V0.10 identity、历史 runtime/optimizer 数值、
  KIR timer/budget、CPU policy、语料或数值门槛。旧 required jobs 仍为失败，必须
  由新代码/协议在最终同一 SHA 的完整十 job matrix 全通过后再关闭 I14/I19。

### Replay 准备流程的组件身份复诊（阶段 11 仍未签收）

- 首次真实准备已在自有 clone 构建出精确 `df8165` 的 0.10 compiler，但 verbose
  identity 校验拒绝了 LLVM manifest 摘要。源码 `build.rs` 明确嵌入的是选定 prefix 的
  `share/ckc/llvm-build.toml` 摘要，不是 `native/llvm/manifest.toml` 构建配方摘要。
  本机两者分别为 `b8b790dcfdd9652b1634d8d50075b1037298ec7cbcf3e7a5fefabb55d1f84874`
  与 `bdfc1199416b70411d9e0c53faf3f06602f71192dab67774c21420c1564510c3`。
- 修复只把期待值绑定到真实安装组件清单，并在准备前后核对该摘要不变，bundle 与
  Rust loader 也记录/验证该字段；原 version、ABI、target 和四个 adapter 检查保留。
  不修改固定编译器源码或 LLVM version，不采集或更新任何性能 baseline 数值。
- 首次失败 driver / preparation log 摘要：
  `4d28da0f502959dcd915e94a5835875970f7910dcd1361fd9961ff573289dc1d`、
  `87e09eacea48a3a06ef337a734e9c0c744b2ae5ea0c99ba708714fc384d91450`。
  此时没有成功的 replay manifest 或性能报告；后续仍需实际生成/校验完整 bundle。

### Replay Task 1：独立准备与完整性校验（尚非运行时验收）

- Rust loader 正例先 red，再加入严格 TSV/固定 identity、源 manifest/recipe/四 adapter
  集合摘要、实际 compiler/library 大小和 SHA-256 校验。缺失/重复/未知字段、模式、case、
  错误版本/target/CPU/组件清单、非 hex 摘要、路径逃逸、symlink、等长篡改与缺文件均拒绝。
  Python 准备验证同样先 red，覆盖四份固定 adapter、四份 CK source、actual verbose
  compiler identity 和拒绝覆盖现存目录；Rust/Python recipe 算法有独立计算对照测试。
- 正确绑定安装组件清单后，在自有 clone 中实际构建 0.10 编译器并生成八份 O3/baseline
  libraries；原 baseline worktree、main、固定源码/数值均不变。独立 Rust driver 对当前
  recipe/adapter/组件清单重新计算摘要，完整读取 bundle，确认八份非空产物。此时没有
  调用 kernel 或生成性能报告，不替代后续交错采样/差分/门禁。
- 实际 compiler SHA-256 `c7c7878e22ac8dd221cf20b9d53dfafc9a4e230200216a4d3ad7fd36381945bb`，
  bundle manifest `465de9f73331ed43ff791f529a5dcab6c32c90373b4c07507c17f963f129bbeb`，
  approved source diff `a68153e19d34f3f89cbe6a2c903537d69d3aebcbbd1c59be012fbaeeb3f1e933`。
  driver / preparation / independent verification logs：
  `0d55b3f4a849c24946aadf39ef088a07ff569c52898dd1ad7c6940a551f1e2c2`、
  `100c7040883927b997fcfac1fbab06af821eca546932f21355611601e1ba3357`、
  `39bb789a9a1dcb0135450f0e8f746d2cb11fc8c37b6e3cdf0984d690802a82fa`。
- `cargo +1.90.0 test --locked --test performance` 13 项通过（内含四项 Python preparation
  tests）；该 target 的 Clippy `-D warnings`、fmt/diff 通过。测试日志 SHA-256
  `f7d8c863e0b145832beb429902dd02ea89f34f57a6147d16e54babbcdb946561`。
  自审范围为准备/完整性边界；Task 2–5 的采样、schema-6 reader、CI 与最终验收仍待执行。

### Replay Task 2–4：同进程采样、schema-6 reader 与 CI 接入（完整门禁待验）

- 纯调度测试先观察固定顺序不能满足第 2 轮轮转，再观察空执行器不能记录调用/传播错误，
  实现后通过位置均衡、精确 3/20 轮执行顺序、各流样本归属、warm/timed 任意调用报错
  立即停止测试。调度不读任何耗时；所有通道共享每 kernel 的输入、seed 和结果 oracle。
- harness 在预热前准备两模式与两份独立冻结 Clang C 编译产物，加载实际固定 0.10
  libraries。每次 cold/warm/timed 调用均校验 status/result。优化器计时函数、固定历史
  数值、语料和全部 95%/10%、3%/8%、raw 97%、2x/3x 阈值均未修改。
- schema-6 reader 对实际 TSV/compiler/library 字节重新校验，报告必须给出八流样本和
  精确顺序。九项 Python 测试逐项注入真实 I14 数字、独立 geometric/individual 回退、
  原全部负门禁、错误历史/MIR 分母、篡改 bundle 与 report 身份、NaN/Inf、缺样本、
  重复键、库缺失/变更/symlink 和路径逃逸；所有预期均通过。原 Rust checker 测试入口
  改为调用该套测试，未删除或放宽旧门槛，去除了只接受合成历史 baseline 的测试耦合。
- 两个 CI performance job 使用完整 Git 历史，在计时前准备独立固定 compiler，上传
  首次报告、原始日志、compiler、manifest 和 32 份实际库。诊断只检查这些库，不再
  另行编译/计时另一份 baseline，也不导出可能为空的 `.text`。CI source-order 先 red
  后 green；现有十个 CI contract tests 全通过，矩阵仍为原十个 required jobs。
- 使用当前 recipe 重新独立准备 bundle，manifest SHA-256 为
  `fe3293d402083be979f5b9f48992d03b498046006ec7245f3e8819afe997baed`，preparation log 为
  `015b1d90cadee83c23bb39f641340bbf9ac157d84f7bc452e6484aa46e7a2204`。
  一次实际 quick 功能运行完成四 kernel × 八通道调用，真实 compiler/32 库的 reader
  完整性核对和诊断脚本均通过；其 report SHA-256 为
  `ff735664872bd4e7d960dd7139bce9ad333da96eff73a1e51f06a4d5a520eace`，benchmark log 为
  `54f356b6f02fe8508116a97a80ae300a52017e5a0723fc7e6524bae4b26eea9e`。
- 严格 gate 对该 quick report 按预期 exit 1：warmup 1 不满足固定 3。完整性日志
  `47bf0b7bb9432f6636a6ec5d50018f33185eb3a3223faf0a5083a6c229726e23`；预期拒绝日志
  `0b54b4d9b6c52980afa4c77cc06ca89c56fbb717484a96b759b6ec4e9ab8b957`。这不是性能验收，
  未用 quick 值修订 baseline 或签收 I14/I19。首次完整 gate 必须在提交后执行。
- 定向 performance 17 项（内含 Python checker 9 项、preparation 4 项）、Native bench
  compile、定向 Clippy 均通过。复审确认报告只按实际 replay 样本归一化、不以库字节
  相同替代吞吐门禁、hash 不冒充不可信编译器签名；后续完整本地/同 SHA CI 仍不可省略。

### Replay Task 5 本地完整功能验证（正式性能/远程待验）

- 默认 468、全特性 591（Native 94）、release 单元 53、release IR 58、all-target/all-feature
  Clippy、fmt/diff 全通过。三个日志 SHA-256 为
  `1642755afd851c6be33d658a7d2cb1d3b6db4f325c48811a13a0a7d48ddf6330`、
  `9824de3c8cbfc3101fadee93c9b194c2fb50c1838ff6ad8615dee73281c317b3`、
  `4aab7e07bce26e3ff8c5a258f4eb58c9ecadf452cadb964fac60a96c00b41524`。
- release Native compiler 构建、generated differential 3、IR mutation 10、pre-LLVM
  fact audit 7、release verifier-cache 5、真实 Native artifact audit 与 hardened JIT
  audit 全通过。实际 compiler 为 `ckc 0.11.0`、Native ABI 1、Runtime ABI 2、LLVM
  22.1.8，安装组件摘要与 replay 相同。licenses 命令 exit 0；审计日志摘要为
  `1442d2edbca71b3d155d8f0fd5c3a31e2add816003ccdd66127e4c1c0d30f00f`。
- 本机 sanitizer 脚本只确认已记录的 Darwin capability unavailable，没有运行或通过
  ASan/UBSan/LSan；Linux native-integration 的所有 sanitizer 门仍必须通过。
  这组功能/身份验证不等于新协议的性能验收，I14/I19/I21 的远程判定保持打开。

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
- 仍在运行旧 `c91c2c0` 的 run `33288920505` / Intel job `99196977382` 后续完成：
  fact audit 7/7、Native 92/92、CLI 21/21 与 release build 均通过，仍因实际 compiler
  `code object is not signed at all` 失败。其完整 log SHA-256 为
  `f5cc267a3ee2c888d0f762fac1aa96a14faa1675eec038157b91964dbe8ff020`。
  这是修复 `02c4978` 之前的相同反例，不是已修复分支的新失败；也不能视为最终
  签名/hardened 审计通过，必须在最终完整矩阵中重新验证。

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

## I17：release 验证缓存把未核验的 pass-preservation 声明当作事实

- 总验收逐项复查 `9429896` 时发现，I05 引入的 `VerifiedKirState` 只在 debug 保存完整
  KIR/proof/elimination/contract 快照；release 在 `changed=false` 时直接接受 cache hit。
  这与规范“analysis output 与 pass-preservation claim 未验证前均不可信”冲突，不是允许
  为编译耗时门槛作出的折中。
- TDD 在同一真实 pass manager 上注入四种未报告变更：不存在的 SSA operand、stale
  ProofCertificate、丢失 guard ProofId、stale trusted contract fact。旧实现的
  `cargo +1.90.0 test --release --locked --lib verifier_cache_ -- --nocapture`
  为 1 pass/4 fail（exit 101），四者均错误接受；debug 为 5/5。相同状态的正例始终通过。
- 修复在所有 build profile 保存并逐字段比较完整已验证状态，只有状态完全相同才复用
  既有证明；不匹配即运行原独立 verifier。没有修改证明规则、忽略 mutation 或信任 pass
  的布尔值。修复后 debug/release 均为 5/5（exit 0）。
- 新 CI contract 先因缺少 release 命令而失败，随后 quality job 增加该命令，契约转绿。
  双语 optimizer 文档和阶段 05/11/总验收补充同一约束。原 2x/3x optimizer latency 门槛
  保持不变，修复后的完整性能和远程矩阵仍须验收，不能引用 I05 的旧性能结果替代。
- 本轮本机 fmt、all-feature Clippy、默认 345 项/全特性 466 项测试、release build、
  compiler/artifact/JIT audit 全部通过；release 故障注入 5/5。最初全仓测试因本地内部
  checkpoint 目录触发既有 repository layout gate 失败；记录移至仓库外后重跑全套，未
  修改、忽略或放宽该 gate。
- 完整本机 performance 原门槛通过：unchecked Clang mean `0.9966`、V0.10 ratio
  `1.0043`；checked 为 `0.9917`、`1.0067`；proof ratio `1.0364`；optimizer
  suite-median ratio `1.0294`。六项 individual 全通过，Dijkstra 为
  `745916 / 350000 ns`。这证明无需用 release 免检来满足本机 latency 门槛，远程仍待验证。
- 默认/全特性日志 SHA-256：
  `70c88f5314dc8e2b79c367c9b341c248522821d8f3bd26e64c8657157046d8da`、
  `0d45a11b22d3b61789847991b406a96624ed6982f76bf0fe04ceb428c3a14044`；
  schema-5 performance report：
  `68633dece0da43328bfaa376e1ccb465d059db6516f85172771314191988328b`。

## I18：O1 propagation 与 O3 induction simplification 的实现验收缺口（O1 已复验，O3 未解决）

- 总验收复查发现 `run_sccp_range` 只生成 `ScalarAnalysisResult`，结果除了计数没有
  被改写消费者读取；O1 无 guard 的函数甚至不执行分析。`induction-simplify` 直接
  `record_current_pass(..., false, ...)`，没有对应 transform。仅验证 pass 名称顺序不足
  以证明阶段 05/07 要求的实际优化已经实现。
- 可复现 O1 反例：`let a: i32 = 20; let b: i32 = 22; if a + b == 42 { return a + b; }
  return 0;`。`emit-kir -O1` 仍含两个 `Add.modular`、一个 `Eq` 和两分支，没有实际常量
  传播。LLVM 后续可能优化这一程序，不能替代设计要求的 target-neutral KIR pass。
- 重新打开阶段 05/07 相应验收，按原设计增加实际 rewrite 的正例/近邻反例并执行 TDD；
  不删除规范中的 SCCP 或 induction simplification，不把空 pass 改名后视作完成。
  在补齐前，前十阶段的历史通过记录不能证明整个候选已满足设计。

### I18 第一部分：已实现的常量传播事务（阶段仍未关闭）

- 新增真实 KIR 改写：modular 整数链、整数 Copy/比较、每条输入边均相同的常量 phi
  的消费者；没有用 LLVM 优化结果代替 KIR 断言。每个新增正例均先观察保留原操作的
  预期失败，再实现对应变换。
- 闭合证明增加 CopyTransfer、IntegerComparison、PhiJoin。独立 checker 检查实际
  SSA operand、整数类型、完整 predecessor edge 集合、精确结论和 rewrite binding；
  全部提案验证成功后才写回。错误替换值、伪造标量/比较结论、过期操作/Copy 与缺失 phi
  输入边均拒绝，且 KIR 保持原样。证明仅用于该改写前快照的事务，不冒充改写后的有效事实。
- 传播使用原有固定 KIR-size 预算；超预算丢弃该函数整个尚未提交提案。预算 0/1/4 的
  原实现均会提交提案，新增 red test 先证实缺失约束，再加入保守撤回。
- 四种整数位宽的 wrap、不同 phi 输入、循环入口/回边不同值、checked overflow/print
  顺序与 strict-f64 保守性都保持验证。没有删除 checked guard 或扩展浮点优化范围。
- 这不是完整 SCCP 验收：路径/契约范围驱动传播、条件边剪枝和 phi 本体替换仍需实现并
  复核证据失效；O3 induction simplification 也仍未完成。阶段 05/07 保持重新打开状态。

### I18 第二部分：范围证书与检查消费者（阶段仍未关闭）

- 入口 contract range 与比较边 refinement 通过每条输入边的 PhiJoin 驱动实际整数
  comparison 改写和安全检查消除。TypeBounds 只能声明真实整数类型的完整范围；
  ContractRange 必须由支配其使用位置的原始规范化契约推导，不能凭 analysis result
  自证。纯数学区间运算共用 helper，但 scope/provenance 与 SSA binding 独立核验。
- 每个 proof step 带 checker 内部推导出的作用域。分支证据只能绑定确切 predecessor/
  target/taken edge；两条指向同一块的分支边仍逐条区分。Binary/Copy/comparison 在
  实际定义块核验前提，PhiJoin 逐边核验，GuardSafety 在被删 guard 的 condition 定义
  处核验。缩窄 TypeBounds、伪造 ContractRange、分支证据倒灌前驱、同 target 两臂
  混用的故障注入均拒绝。
- 溢出、除零、有符号除法溢出、定长 slice 索引的范围正例先观察未删除 guard 的 red，
  再加入闭合证书消费。近邻保留可能失败的检查：另一分支、零除数、`MIN / -1`、
  `index == len`。可执行 C 对照遍历 O0–O3，检查边界数值、错误码及失败时结果槽不变。
- 跨阶段 red 暴露 GVN 删除新范围证书引用的重复常量。修复为从 live certificates
  提取完整指令依赖，供后续 scalar folding/GVN/LICM/DCE 保留；每份 guard 证书只投影
  所需依赖 DAG，另用未使用的常量验证不保留无关死代码。任何非法证书都返回编译错误，
  不再把非 GuardSafety-root 的验证失败当作普通 unknown。
- 原完整 SCCP 与阶段 07 的未完成项不变：真正的 sparse worklist、条件边剪枝、phi
  本体改写及对应失效/重建、实际 O3 induction simplification。不能以本批范围消费者
  或历史 pass-order 测试替代这几项验收。
- 最小有符号字面量探测发现兼容回归：既有 semantic KIR 将 `-2147483648` 表示为
  positive-magnitude literal 加 checked negation；新常量分析把范围域无法表示该 literal
  的情况升级成编译错误。i32/i64 最小写法的 red 确认 O0 有产物而 O1 错误退出。
  修复仅使该 literal 不产生 scalar claim、保留全部原始 operation/guard，不重新解释
  字面量或改变前端规则；green 严格比较 O1 artifact 与 O0 artifact 完全相同。

### I18 第三部分：SSA-use 稀疏范围工作队列（阶段仍未关闭）

- 以真实合法 KIR 的 79 个单指令块构造反向布局常量链，固定 `3 * instruction_count`
  预算。旧全函数扫描耗尽预算，0/39 项改写；稀疏工作队列实际完成全部 39 项，且独立
  checker 通过、返回值实际改为常量 40。没有增大默认预算或依赖墙钟时间。
- 第二个 red 将比较所在块放在后继之后，但保持 CFG/dominance 完全合法。原实现首次
  访问 phi 时缺少比较的另一操作数，记录宽范围后不再更新，两个路径局部比较都未折叠。
  修复把比较的两侧操作数纳入相应 phi 的使用依赖；后到范围唤醒 phi 和下游消费者。
- 范围不变不再传播，也不留下新重复证书步骤；范围变化保留闭合、按先后引用的推导。
  新增算术传播与相邻非恒定范围对照、重复运行证书一致性断言，原全函数预算撤回、
  作用域故障注入及 O2/O3 证书保留测试仍执行。队列只改变求值调度，不自证优化结论。
- 这只关闭 sparse evaluation 缺口，不表示完整 SCCP 已验收：条件边剪枝、phi 本体改写、
  CFG 证据失效/重建、checked 下游传播核查以及阶段 07 实际归纳变量简化仍待完成。

### I18 第四部分：常量 phi、CFG 重接与导入事实失效（阶段仍未关闭）

- 首个 red 直接返回常量 phi：消费者虽可折叠，原 block parameter 仍存在。现以闭合
  `PhiJoin` 核验全部输入，在写入任何指令前检查精确替换值与整批新 ID 空间，然后保持
  ValueId、生成 ConstInt、删除 phi 参数并修复每条输入边。五项故障/边界回归覆盖错误值、
  缺失输入证据、ID 耗尽、有效证明依赖保留及预算耗尽时整批撤回。
- 条件比较折叠后原 Branch 和不可达调用/print 仍存在的三个 red，现由检查消除前的
  propagation/CFG 不动点闭环解决。每轮改动先结构验证，再重建存活契约导入；失效的
  分析事实不升格为契约公理，存活 inline clone 保留来源祖先，旧临时 proof 不跨 CFG 复用。
- 空 if 的 red 为 4 个块，现转发两侧空块的标量与 memory 参数，合成相同边后为 2 个块。
  保留具有非局部 scalar/memory 使用或契约绑定的参数定义；不移动任何可达指令。
  新 C 执行对照覆盖 O0–O3 与 checked/unchecked 的 phi 参数交换、store 顺序和常量路径。
- DCE 的新增 red 表明描述符删除后遗留 RawSlice/Subslice region。现仅清理无存活定义、
  无 place/memory/父子关系引用的派生 region，仍保留无用 subslice 的失败检查。
- 本批不等于阶段 05 完整签收。后续仍需核查布尔 Copy/phi 的条件传播、checked 双结果
  的下游分析，以及本阶段最终验收；阶段 07 与性能阻断也未关闭。

### I18 第五部分：布尔与 checked 传播，阶段 05 签收

- 布尔 Copy/取反未折叠和全 true phi 未剪枝的实际 red 已解决。新增独立布尔证书
  绑定 value、真假、定义 scope 和全部输入边，不以 optimizer 的布尔缓存作验证依据。
  整数比较结果进入同一稀疏队列；不同/未知 phi 输入、循环翻转值均保持原语义。
- checked 二结果指令的首结果只在证明 failure=None 时提供下游范围，保留 checked
  producer，guard 仍单独携证删除。safe `(20 + 22) == 42` 与路径 `(n + 1) < 9` 的 red
  均转 green，不能证明安全的相邻分支仍保留失败检查。
- 真实分析反例 `% 0` 暴露 failure 分类晚于区间构造；复诊确认并非语法或运行时契约
  问题。现先返回 unknown+Always，消除非法区间错误和除零的虚假精确结果。非零常量
  `%` 的另一个 red 通过精确余数计算解决，负数符号、四种整数类型和两个算术模式均验证。
- 五项布尔证据故障/预算测试及全模式 C 执行对照通过。C 对照检查短路写入、循环真假、
  首错、失败前后的 store 与结果槽。阶段 05 第 1–9 项全部通过，全量 default/all-feature
  为 409/530，release library 29；哈希和环境见阶段 05 验收文末。
- 本部分关闭阶段 05 的实际传播缺口。阶段 07 的实际 induction transform、loop 证书
  entry/transfer 独立复核与相关近邻反例仍需完成；I14/I19 和同 SHA 全 CI 仍打开。

### I18 第六部分：循环输入与 scalar invariant 证书的实际反例（阶段 07 仍打开）

- `while i < n { if flag { i = i + 2; continue; } i = i + 1; }` 的合法 checked KIR
  被报告为 step=1：旧 detector 覆盖先前 latch，只使用最后一条 transfer。现逐条核对
  所有入口初值和回边步长；混合步长与循环内分支重赋值保持 unknown。
- 旧 `canonical_loop_param` 与 bound normalization 依据 slot 名称识别同值；现沿实际
  SSA 输入和 Copy 遍历，所有终端都必须到达同一 origin，循环转发必须有真实 origin。
  每条分支边保留，即使二者有相同 predecessor/target 也不合并。嵌套及一致多 latch
  正例继续通过，不通过禁用自然循环分析规避反例。
- 独立 scalar `LoopInvariant` checker 存在可复现错误接受：合法 KIR 的真实回边传递
  `i + 1`，另有未参与回传的 `i + 0`；证书却可借后者证明 `i` 始终为 0。修复逐条核对
  实际 backedge argument 必须等于 certificate transfer 的首结果，并验证类型和
  failure=None。错误绑定拒绝；把真实回边改为 `i + 0` 的合法近邻接受。
- 这两项针对性 red 已转 green。仍需完成实际 `induction-simplify` transform、guard
  loop certificate 不调用优化分析的独立验证、irreducible/fixed-budget 等原阶段 07
  要求；不得用本批局部修复签收阶段 07。

### I18 第七部分：独立 guard 检查器与分支边支配反例（阶段 07 仍打开）

- 恢复后复诊确认 guard checker 调用 `analyze_natural_loops`，违反已批准的独立检查器
  约束。新增架构回归实际失败；实现改为局部 strict-bound 前提检查，不再调用该分析。
- 实际 red：把合法 u32 循环 header 的 false edge 改成与 true edge 相同，结构 verifier
  仍通过，但旧 checker 接受 `i + 1` 安全证书。此时循环体也在 `i >= n` 时执行，旧
  “then target 支配使用”并不代表 taken edge 支配。现删除具体 taken edge 后执行
  reachability 检查，必须无法到达使用位置；该反例拒绝，原正确 CFG 接受。
- 实际 red：只重命名 canonical slice loop 的 block parameter slot，不改变任何 SSA
  输入，合法证书却失效。旧代码以源名匹配函数参数；现沿所有真实 phi/Copy 输入核验
  identity，循环转发使用 visited set，模糊输入保守保留。另加不同 slice 故意使用相同
  slot 名的有效 KIR 近邻，不能借用原 slice 的契约消除检查。
- `i < bound` 的当前路径事实直接证明同类型 `i + 1 <= bound <= MAX`，u32 索引非负，
  再结合同一 slice 长度/契约证明 bounds；此处不声称循环不变量，因此不需要让
  optimizing induction analysis 自证。真正 `LoopInvariant` 形式继续检查全部
  entry/backedge transfer。本修复不改规范和门槛，不删除原 canonical loop 正例。
- 新增四种整数宽度 × O2/O3 的严格/非严格比较、两种加法方向、步长 2、重赋值、
  不同 phi 输入的近邻；C 执行对照覆盖 O0–O3 × 四种 safety mode 的整数极值、零次
  迭代、checked 首错、此前写入与失败后输出槽不变。
- 此节不签收 actual induction-simplify、irreducible/budget 或阶段 07 全部任务；
  I14/I19/I20 性能失败仍打开。完整本轮验证证据见阶段 07 acceptance 的局部复验记录。

### I18 第八部分：真正的携证归纳简化（阶段 07 尚待完整复验）

- 实际 red：相同初值、相同步长的 i/j 计数器在 O3 仍有两条 modular Add，命名
  `induction-simplify` 没有任何改写。现通过闭合 `InductionEquality` 证书合并
  loop-carried phi，原测试 O2=`2` / O3=`1`，pass record 确实 `changed=true`。
- proposer 构造等式依赖；checker 不调用 proposer 或循环优化分析，而逐项核验
  同类型、真实 producer、所有入口/回边与相同 arithmetic semantics 的 transfer。
  证书丢失 transfer pair/producer、初值或操作改变时拒绝；错误 replacement/ID 耗尽
  均在任何修改前失败，不产生部分结果。保留原 ValueId 的 Copy 与参数重接避免改写
  契约身份，受已有 phi 证书依赖的参数不删除。
- 新增四种整数宽度 × 两种 overflow mode × 升序/降序/多 latch 的正例，以及不同
  初值、不同步长、单一 continue 路径漏更新的反例。100 个固定预算配置覆盖从
  无候选到部分 proposal 后耗尽，耗尽的函数必须逐字节未改；不使用 wall clock。
- C 对照覆盖 O0–O3 × 四种 mode：整数极值/模回绕、两次更新中间 break、checked
  首错、此前写入与失败结果槽。原三个 fixed-seed kernel 保持不变，追加三个包含
  嵌套等值计数器、break/continue 和实际 slice 访问的 kernel，沿用相同 seed，交由
  原 C/WASM/Native 对照执行器运行；没有替换或削弱原语料。
- 本节修复的是 actual induction-simplify 缺口。自然循环分析的 irreducible fallback、
  完整分析预算与阶段 07 全任务仍需验收；I14/I19/I20 和全部最终 CI 门槛仍打开。

### I18 第九部分：不可约循环与固定预算回退（阶段 07 尚待完整复验）

- 实际 red：多入口循环的 `irreducible_blocks` 始终为空；自回边循环把 preheader
  错误纳入 body。现移除 dominance backedge 后，对剩余图执行确定性、非递归 SCC
  遍历，识别不可约核心；也覆盖单一外层 SCC 内隐藏的多入口内循环。自回边在 header
  停止 predecessor 追溯。不可约函数不发布 natural-loop/induction 候选。
- 固定预算实际 red：任何配置都返回完整结果，没有耗尽状态。现 dominator 使用与
  structural verifier 相同的实现，但 analysis 单独计量矩阵与迭代工作；后续 SCC、
  loop nesting、归纳候选及 SSA forwarding 也计量。任一步耗尽都丢弃整个函数的部分
  分析结果，verifier 本身仍完整执行。预算只由 KIR 大小/固定配置确定。
- 新测试还复现块存储顺序影响迭代预算；按 block ID 调度支配迭代后，相同 CFG 的
  分析与回退保持确定性。嵌套自然循环预算扫描与不可约 SCC 全预算扫描验证结果只有
  完整值或 unknown，不泄漏部分候选。原 nested、多 latch、实际归纳改写正例保留。
- pipeline 对不可约 CFG 的真实测试验证 LICM/induction pass 都不改写，artifact 仍
  通过 verifier。CLI 实际 red 是缺少回退原因；现 `--explain-optimization` 输出
  函数/pass 与 `fixed-kir-budget-exhausted` 或 `irreducible-control-flow`，也覆盖
  induction 搜索预算。CLI 重复运行字节一致，不伪造 guard ID 表示分析级原因。
- 本节不替代剩余阶段 07 的 LICM/完整语义自审，也不覆盖 I14/I19/I20 的性能失败。
  规范、语料、性能阈值和必需 CI job 均未调整。

### I18 第十部分：LICM 的真实不变表达式与提前执行边界

- 实际 red：原 LICM 正例只要求 `hoisted_instructions > 0`，常量移动即满足，但
  `a * b` 仍在循环内。加强测试直接定位 Mul 后失败。根因是前端保留了 a/b 的循环
  phi 转发，而 LICM 把这些操作数都视为变化值。
- 现沿全部真实 SSA phi/Copy 输入寻找同一外部来源，并在改写操作数前由 verify 模块
  独立核验来源身份与类型。不信任 slot 名或分析自己的结论；声明在不可变 pre-state
  上消费，不跨改写复用，不提升为 trusted contract。保留 ValueId、Memory SSA 和
  存活证明 producer，移动顺序保持数据依赖而非按 instruction ID 排序。
- 原除法/WASM 零迭代测试先通过；加入上述真实不变识别后，旧 modular 分类把 Div
  也提升，测试实际出现 WASM divide-by-zero trap。提交前收紧到确实可提前执行的
  整数 add/sub/mul 等纯运算，Div/Mod 保守留在原位。不是放宽 unchecked 语义或忽略 trap。
- 新增独立来源检查器的错误来源/变异回边拒绝、受保护 producer、逆序 ID/块存储的
  依赖顺序、固定预算耗尽的整函数恢复测试。别名 load/store、递归 call、print 和
  strict float 均有实际指令位置断言；C 执行覆盖 O0–O3 × 四种模式的零次迭代、
  checked 首错、此前写入和结果槽，WASM 覆盖零次迭代及 break 绕过除法/取模。
- 归纳事实新增四种宽度 × 两模式 × 八类方向/步长/严格边界组合（64 组）。实际 red
  是降序 strict `-1` 未报告逐点 wrap-safe；补齐同类型最小值方向的对称规则。
  此事实不自行授权删除 guard，guard checker 的独立局部规则保持不变。
- 一项测试源码误用了保留字 `effects` 作为函数名，已改成 `effectful`；没有更改语言
  关键字或前端解析规则。性能 I14/I19/I20 与完整最终 CI 仍独立打开。

## I20：布尔/checked 传播后的 optimizer latency 门槛失败（未关闭）

- 阶段 05 本批代码通过功能与证书验收后的首次原 performance gate 返回 exit 1：
  `example-dijkstra` KIR median `1456625 ns`，固定 V0.10 MIR 为 `350000 ns`，ratio
  `4.1618x > 3x`。此前 CFG 批次同门槛记录为 `980583 ns`；不同测量轮次不直接等于
  精确归因，新增工作量与测量条件均待诊断，不能先归咎于机器噪声。
- 本轮 runtime 对照为 unchecked Clang mean `0.9987` / V0.10 ratio `1.0022`，checked
  `0.9965` / `1.0019`，proof throughput `0.9830`，这些通过项不抵消 optimizer 失败。
- 原 schema-5 report SHA-256 为
  `19052d5ee224f1dc9e87118a21b38c70d3df09671c9735fa7a8e6a9a4288d89d`，benchmark log 为
  `448f835f23a8f92790624b717fdbc17cbcd67832e1cde2e79381524e09193956`。原始样本已保留。
- 阶段 11 必须诊断并通过该原始 2x suite-median / 3x individual 门槛；不得调整 corpus、
  baseline、跳过独立验证或反复重跑选取 green。阶段 05 功能签收不构成全目标完成。

### I20 首次 CPU 采样与归纳查询索引（仍未关闭）

- 在阶段 07 完整实现 `a617fa1` 上，用 release library 对相同 Dijkstra NativeLibrary /
  unchecked KIR 执行 20,000 次 O3 pipeline。macOS `sample` 的主线程 8,465 个样本中，
  3,664 个位于归纳简化，热点包括反复线性查找 SSA 定义、扫描全部 incoming edges，
  以及每次遍历时分配长度为 0–2 的临时 Vec。原采样 SHA-256：
  `ad42365992ccb532eba6e28936b27c127c4dd9558304784f55f06ddcba125f3e`。
- 归纳 proposal 现为每个不可变函数建立只读定义/类型/参数/输入边索引；边遍历不再
  分配临时 Vec。输入边顺序及同一 target 的两个 branch arm 均保留，搜索工作队列、
  固定预算扣减和完整证书检查不变。独立 checker 不接收或信任该优化侧索引。
  查询等值测试逐项对照原线性实现，并包含缺失/极大 ValueId 和重复 target 的两个边。
- 修改前后 Dijkstra Inspection KIR 逐字节相同，SHA-256 均为
  `3f49f6f77153df15c85ebc3e85318047c9b91e2d9ce65dcac40aca925fbf56f0`；NativeLibrary
  诊断的全部 rewrite 统计也相同。相同 20,000 次驱动的总耗时由 `45.806477792 s`
  变为 `31.702653833 s`；后续采样 SHA-256：
  `82be7738c89e606fd9775e29770c920331c47858d99b4f2a89e657ecd5d8a383`。
- 上述驱动计入输入 clone/结果析构，并带 CPU 采样，不是原 benchmark 的定时边界，
  只用于定位及诊断，不能签收 I20。它也不能解释尚未实现归纳简化时的历史首次失败。
  原始失败、全部 corpus、2x/3x 门槛和独立验证保持有效。
- 默认测试 444 项、全特性 566 项（含 Native 93 项）及 all-target/all-feature Clippy、
  fmt、diff 检查通过。顺序执行的 default/all-feature 日志 SHA-256 分别为
  `471f146811b4e5574ac110b50de4325c84c55ac4aa60eafa0dadbb869985db24`、
  `7aae4b1d621a0588025975c823a230c105c16fbfd34a71a7a5f9c759756908d5`。
- `48c8b12` 的首次完整原门禁仍返回 exit 1：Dijkstra KIR `1707917 ns` /
  V0.10 MIR `350000 ns`，约 `4.8798x`。unchecked Native/Clang geo `0.9974`、V0.10
  ratio `1.0035`；checked 为 `0.9992` / `0.9991`；proof throughput `1.0215`。
  report / benchmark log / checker log 的 SHA-256 依次为
  `cd38406ff6f000aaf45b10f736205bd6b8571b908a8f286c7503c68de2052818`、
  `7ead6f3c0343eaf42ab61e269b208e94714075183807608e69e022cffd38ad95`、
  `b7510b1667981e40d4ed8f6b6a0bf36b3e64d30f5fe3702902a725d3ebf45038`。
- 本任务没有并行构建，但运行中检查发现另一仓库的 release rustc 正在同机占用
  CPU。未干预该进程，保留原报告并注明非独占环境，不以该次时间量化代码回退，
  也不以同次 runtime 通过关闭 I19。后续继续针对已有采样中的实际工作量优化。

### I20 后续重复遍历收敛（仍未关闭）

- LICM 对每个 loop 的不可变 pre-state 建立参数/Copy/输入边查询，跨 loop 重建，
  不缓存已经改写的 Copy 来源。原线性实现保留为测试对照，对全部 fixture 定义与
  缺失 ValueId、100 个预算逐项比对返回值和剩余预算。新增实际同 target 双 branch
  arm 的不同输入反例在重构前后均保守拒绝。原整体预算回退、producer 保护、
  零迭代和依赖顺序规则均不变。
- 独立 checker 仍自行扫描真实 SSA 边，只消除每个 predecessor 的临时 Vec；没有
  复用优化侧查询或已验证来源来跳过证明规则。release proof mutation 9 项通过。
- GVN 将既有的有序 substitution 先复合，再单次遍历函数的使用点及 region 元数据；
  不改变候选、支配判断、受保护 producer 或改写顺序语义。4,096 组三次替换包含
  重复来源、恒等、连锁、交换和极大 ValueId，结果与原逐次遍历完全相同。测试最初
  误用不存在的 `KirBuildConfig::default`，改为显式 checked 配置，没有更改构建契约。
- 两步之后的 Dijkstra KIR 仍与前述基准逐字节一致，NativeLibrary pipeline 的全部
  rewrite 统计也不变。LICM 后与 GVN 后的 20,000 次诊断总耗时分别为
  `32.157167959 s` / `26.715015416 s`；由于非独占机器状态和诊断边界不同，不作为
  门禁或单变量收益结论。相应 CPU 采样 SHA-256 为
  `93aa6122aab2ed9c52287f09b0283e44f3af550c9eb2e229dfd3e7674e59d461`、
  `4053c788a7ac7b51388628139f3f24945e0db4451b77431cae4f6febe41db502`。
- 默认 447 项、全特性 569 项（Native 93 项）、release 单元 42 项与全特性 Clippy、
  fmt、diff 全部通过。default/all-feature/release 日志 SHA-256 依次为
  `2425d978eb6b0baa95109c3e91f2de1e5edb1d67b35bc224c623e1613a794594`、
  `1b3a3daaa4408eea619647bf40eb02f8fe05155f7fd33a714273ccf102f986e4`、
  `291d53de4288fa16ca0aa4153394af042b87cc644eec37045cb5afbe82c2753a`。
  本批尚未取得新的完整性能验收，原 I14/I19/I20 继续打开。

### I20 结构校验临时分配收敛（仍未关闭）

- 结构校验器只读借用每个 SSA 定义的 MirType，不再为每次验证克隆 pointer/slice/
  struct 类型；定义唯一性、类型等值、所有错误消息与遍历顺序保持不变。操作数使用
  直接 visitor，terminator 及其输入边使用借用 iterator，避免每条指令、place 和边
  列表的短生命周期 Vec。每个 pass 的完整状态比较、structural verifier 与 proof
  checker 仍执行原规则；没有缓存分析结论或跳过任何校验项。
- 新增单元逐项覆盖全部 instruction/place variant，与原收集器比较完整有序 ValueId
  列表；terminator 测试覆盖空/有值 return、jump、两个相同 target/相同参数的 branch
  arm，重复值和两条边都不得去重。release IR 全部 58 项及缓存故障注入 5 项通过。
- 默认 449 项、全特性 571 项（Native 93 项）、release 单元 44 项与全特性 Clippy、
  fmt/diff 全部通过。Dijkstra KIR 仍与前述基准逐字节相同。default/all-feature/release
  日志 SHA-256 分别为
  `a91292163a75fb550a2d3d9d42d850a28b9a5f6ca1337be17e9dd55819d7027a`、
  `4f49289858166cd8edaa81abadd8225b15493d5198a6fb64513b65d53ea12060`、
  `b27f0bec4c3c231c5c9f447b59dea058558a100f95564bf11b4efe50b71050b1`。
  这些是功能/校验保持证据，不是原性能门槛通过证据。

### I20 后续结构化查询诊断（仍未关闭）

- `7309cfc` 的首次完整原门禁仍返回 exit 1：Dijkstra KIR `1279166 ns` /
  固定 V0.10 MIR `350000 ns`，约 `3.6548x`，其余五个 optimizer case 通过。
  unchecked Native/Clang geo `1.0024`、V0.10 ratio `0.9985`；checked 为
  `1.0006` / `0.9978`；proof throughput `0.9949`。report / benchmark log /
  checker log 的 SHA-256 依次为
  `dcef25f6de244f0df499b719bd17e31eb8d2639383bf628145e683d2436e4fe3`、
  `ce7a566deb082f395e09e5dc9a911670a5412961be9804187f3e2ca5849f4ba5`、
  `ba4b9bd6b2857f200d7f2bdde9ab80b3d1522ccf330efdd9731007f6c3ec11e9`。
  前后进程快照不能证明整个测量窗口独占；未以此关闭 I14/I19。
- 隔离目录的同源码普通/ThinLTO ABBA 诊断，每轮 10,000 次，分别为
  `12.447123916` / `12.321859959` / `12.320796875` / `12.831959916 s`，全部
  rewrite 统计相同。该驱动包含 clone/drop，非正式门禁；未改变 Cargo release
  配置，也不以约 2–3% 的差异声称解决超标。
- `7309cfc` 的后续 CPU 采样 SHA-256 为
  `679d1d5f29bcc43ec703f4b737990d8341066cefeb208aee3884f5e1a5770d76`，用于继续
  检查 GVN 格式化、归纳查询与结构校验的实际工作量，不改变语义/验证边界。
- GVN 使用借用类型/常量文本的结构化表达式键，避免整条指令克隆与 Debug 字符串
  构造。类型、精确常量拼写、操作符、操作数顺序和 arithmetic semantics 全部参与
  identity；checked arithmetic 仍不作为候选。lookup-only HashMap 从不通过遍历选择
  producer，原 block/instruction/definition 顺序、支配与 effect 边界均保留。
  新测试双向比对旧字符串键的等价类，覆盖全部候选操作符、三种 arithmetic semantics、
  极大 ValueId、循环单步 remap、pointer/slice/struct/primitive 类型和不同常量拼写。
  最初测试使用不存在的 cast variant，已修正为实际 I32ToF64/U32ToF64，再观察仅缺少
  expression-key 实现的预期 red，未修改语言 cast 契约。
- 归纳分析将每个 ValueId 的类型/参数/指令信息合并为一项查询，保留原线性查询对照
  测试、双 branch arm、全部候选及预算扣减。两项之后、尚未修改结构校验表时的
  ABBA 10,000 次诊断为旧 `12.868490917` / 新 `12.326551208` /
  新 `12.207627375` / 旧 `12.591068333 s`；全部 rewrite 统计不变，不能作为门禁。
- 每次 structural verification 重新从真实函数建立本地 SSA definition/type 表，
  仅当编号跨度不超过定义数四倍时使用连续存储；稀疏、极大编号及后续范围外插入
  回到精确 HashMap 查询，不按最大 ID 无界分配。重复定义仍保留原 definition 与最后
  记录的 type，确保后续诊断不变。新测试与旧双 HashMap 逐步比较完整查询结果，覆盖
  空表、连续/稀疏编号、u32 极值、跨函数冲突、重复定义、缺失值与后续插入。
- 每次 pass 的完整状态比较、structural verifier、proof checker 和 rewrite binding
  规则均未改变。默认 451 项、全特性 573 项（Native 93）、release 单元 46 项、
  release IR 58 项、all-feature Clippy、fmt/diff 全部通过，Dijkstra Inspection KIR
  仍为 SHA-256 `3f49f6f77153df15c85ebc3e85318047c9b91e2d9ce65dcac40aca925fbf56f0`。
  default/all-feature/release 日志 SHA-256 依次为
  `3f1de8418d882b25a497a68895da9aa6dfc54d613a7552cf81c62329efdc4e38`、
  `8cda6a7ebdf98eb74ec2d8a8735d2257a6ba04f894751b724958f60b209cc47c`、
  `c5fde4b2f39b8e61b74963057c44011bceb27f0ce7daee3ca6bec80dc3481cfa`。
  本批功能验证不签收 I14/I19/I20；原始失败和所有性能阈值继续有效。

### I20 phi 查询借用与自支配检查（仍未关闭）

- `9774f07` 首次原始完整门禁仍失败：Dijkstra KIR `1078458 ns` / V0.10 MIR
  `350000 ns`，约 `3.0813x`，高于固定上限 `1050000 ns`。其他五个 optimizer
  case 通过；unchecked Native/Clang geo `0.9998`、V0.10 ratio `1.0010`；checked
  为 `0.9983` / `1.0000`；proof throughput `0.9979`。未重跑相同版本挑选结果。
  report / benchmark log / checker log SHA-256 依次为
  `9f3798aa8f97bc40589ff91c470fbd204557a7ae7bd759778df6d1c394af8787`、
  `64e3aa66870f53da74831c68d53d4ca5125424e80c26b7ff5dcc18236f3e8a4a`、
  `40e29c83308065894bfcd06e8683e4d6dcc0bcfdc0336326ca977ead64def1b3`。
- GVN phi 查询借用原输入边的参数 slice，并流式比较，避免临时边 Vec、参数 clone
  与每次合流收集。保留双 branch arm、输入顺序、单步 canonical 映射和原收敛条件；
  测试逐项对照原收集实现，含 Dijkstra、循环/continue、同目标两个边、不同或缺失
  输入的保守查询行为。测试在重构前后均通过，没有改变 SSA 合法性检查。
- scalar/memory block parameter 在自己的实际 use block 自支配，直接应用该恒等关系，
  跨块仍查询完整 dominator 集。新测试对照完整 dominance，包含不可达块、自身、
  可达不同块和未定义来源；重构前后结果相同。未改变定义唯一性、instruction 使用
  顺序或 independent proof checker。
- 默认 453 项、全特性 575 项（Native 93）、release 单元 48 项、release IR 58 项与
  all-feature Clippy、fmt/diff 全部通过。Dijkstra KIR 与前述 baseline 逐字节一致，
  SHA-256 仍为 `3f49f6f77153df15c85ebc3e85318047c9b91e2d9ce65dcac40aca925fbf56f0`。
  default/all-feature/release 日志 SHA-256 依次为
  `f53bb944931f565182d237968276706651cbb1093b3721c3aa45b9b93c143cee`、
  `55d2046303340301294f2f33476e8d3eb979340ddf8e038580fe8818713d6977`、
  `4a7ca3a544e94e1f8c4368d87b2ca92c1c57c81b1bc0be5eca9b98f84a13cdfe`。
  本批不修改性能协议、语料、统计方式或门槛，I14/I19/I20 仍须后续正式验收。

### I20 `7611fa6` 首次门禁与无用 phi 复诊（未关闭）

- 原始完整门禁仍失败：Dijkstra `1112250 / 350000 ns = 3.17786x`；其余五项
  optimizer 门通过。unchecked Native/Clang geo `1.0001`、V0.10 ratio `1.0007`，
  checked `0.9903` / `1.0082`，proof throughput `1.0093`。没有重复同 SHA 选取绿灯。
  report / benchmark / checker 日志 SHA-256：
  `f0fae936759c4e0891bc5da0c8e7129d65669af1751af7d9cf3dbea3e492fe63`、
  `c631d38f756955ff546606dd8b60cd8e284dfbf860217811a6e8a11601f8d0f9`、
  `bd1866fe898f0f4b3b53084e9a46028c744e4275a00a39505127eb7cecd5088f`。
- 只读 SSA 依赖图诊断发现 Dijkstra 输入函数 456 个标量 block parameters 中有 248 个
  无根参数，O3 后 436 个中有 241 个；`should_relax` 输入和 O3 都为 16 个中 10 个。
  诊断将全部指令操作数、分支/返回值、区域元数据和契约 binding 作为根，沿全部实际
  phi 输入反向传播；此语料没有契约。它不修改 IR，不是性能或正确性验收证据。
- 复诊选择细化已有 pre-proof CFG phi repair，而不是改变 O0 SSA builder 或只在
  最终 DCE 清理。生产实现还必须保护全部 fact predicate 引用，保留双边输入与所有
  指令/Memory SSA，详见 `../implementation/11-ssa-phi-pruning-plan.md`。该补充计划
  自审无逻辑阻断，先单独提交，再 TDD 实现；不改变任何性能或最终 CI 门槛。

### I20 无根标量 phi 实现验证（性能待验）

- 计划提交 `6869b3a` 后先执行新回归，得到预期 red：O1 仍有 3 个 `unused` 参数，
  期望为 0。实现只在 pre-proof CFG 阶段标记真实 ValueId 依赖，过滤 dead block
  parameters 与对应的所有 edge args；指令、公开参数和 Memory SSA 原样保留。
- 单元测试覆盖无根环/幂等、双 branch arm、block 存储顺序、元数据/契约根及全部
  fact predicate、错误目标/arity 的原子 no-op。旧 empty-branch 测试的冗余 `flag`
  内部参数断言已按补充计划改为精确 `n` phi/return，增加公开双参数不变断言，
  完整 Memory SSA 传递断言保持；并非取消活跃参数或效果检查。
- 默认 459 项、全特性 581 项（Native 93）、release 单元 53 项、release IR 58 项、
  all-feature/all-target Clippy `-D warnings`、fmt/diff 全通过。命令均为 Rust 1.90.0，
  全特性使用既有 pinned LLVM/Clang 22.1.8 和 TypeScript oracle，默认/全特性串行。
  default / all-feature / release 日志 SHA-256：
  `7a7c8f2eda1cef2673bb81dbf6020aa8631ed8f9181be1098848a65f7539c637`、
  `f9bb49a8036bbaf0a0b824f1881bcde4462d971b897dd7d9f12757c68c23d318`、
  `a532f0d2d5d131d001d4de651a02e21462052a5675fc30d47e35490046d8bac4`。
- 同一只读诊断中 Dijkstra O3 phi 从 436 减至 195，无保守死 phi；`should_relax`
  从 16 减至 6。原有 DCE 因无用传参消失进一步清理纯指令，Dijkstra 指令从 51 减至
  41。因此本批不再宣称 KIR 字节相同，而以结构校验和完整 C/WASM/Native 差分证明
  语义保持。自审未发现未保护引用或证据失效缺口；所有性能阈值和原失败仍保留。

### I20 `930f18d` 首次原始完整门禁（本地关闭，远程待最终 SHA）

- 原命令 `cargo +1.90.0 bench --features native-toolchain --bench ckc_perf -- --task check --cpu baseline`
  和未修改的 checker 均 exit 0。Dijkstra `789625 / 350000 ns = 2.2561x`；其余
  pricing `127792 / 83334`、pricing-soa `70458 / 68583`、f64-kernels
  `174417 / 162375`、proof `46375 / 40375`、example-pricing `62833 / 66709`，
  suite-median ratio `1.1114`，全部在原 2x/3x 门槛内。
- unchecked Native/Clang geo `0.9990`、V0.10 ratio `1.0019`；checked
  `1.0072` / `0.9912`；proof throughput `0.9993`，individual gates 全通过。
  这是该代码 SHA 的首次完整测试，没有重跑挑选结果；测量期间本任务无其他编译/测试，
  未声称整台共享主机在全时段独占。I14/I19 原始远程失败仍独立保留。
- report / benchmark / checker 日志 SHA-256：
  `c175c70fca8b0edfb6276341c083ab444b3ed855a527bd2dcc47caab718ef71d`、
  `c263c4b99414fd8859cba8ef486449dcdffd500039423ece8b245ed48d0e5cc1`、
  `500e3da3309f33fd121113e27f8656817d900f478cdd5d7c07c6eb16eef2cd87`。

## I19：本机跨 checked 模式 proof-loop 吞吐门槛失败（未关闭）

- I18 工作队列改动后的首次完整 performance gate 返回 exit 1：unchecked Native median
  `4663458 ns`，checked `4904542 ns`，吞吐比 `0.9508447 < 0.97`。原始 20 项样本与
  schema-5 report 已保留，不能仅以其他门槛通过覆盖该失败，也不能重跑直到偶然转绿。
- 用 `git archive a46717ccf4254f291cfdbdadecf88ed31547c652` 的独立源码重新构建 parent
  compiler，并用相同 LLVM 22.1.8 manifest、baseline CPU、O3 和 safety mode 生成
  proof-loop objects。每个模式的 parent/candidate 对象逐字节相同：checked SHA-256
  `faf2d36e3cc47dc241b8511f86c6c9c6125580348677f4ef8cde512e2c76ea80`；unchecked
  `b0f06b7a1fa6169d9927be0e13c7fd3faa27d247f593378c3f0b32ffc5034ff4`。这排除了本批
  SSA 调度改动导致该 kernel 机器码变化，不能据此宣布性能门槛通过。
- 同轮 Clang proof medians 为 `4663417 / 4906750 ns`，跨模式吞吐同样降至 `0.9504085`。
  harness 先跑完整 unchecked suite 再跑 checked suite，两次 proof 之间还测量其他 kernel。
  复诊时观察到系统 XprotectService、mediaanalysisd 与编辑器占用多核 CPU。跨时段机器
  状态是有证据支持的候选解释，尚不能把具体调度/温度原因当作已确定事实。
- 未终止任何用户或系统进程，未修改 benchmark 的样本数、执行顺序、阈值、语料或固定
  baseline。保留这项失败，继续诊断并等待正式稳定环境证据；I14 的远程 x86 基线问题仍
  独立打开。最终仍必须在同一最终 SHA 通过全部原门槛。

- 远程 ARM job `99196977301`（run `33288920505`）也以 proof 吞吐门失败：Native
  unchecked/checked 为 `7186124 / 7902379 ns`（`0.9093621`），Clang 同项为
  `7240401 / 7913075 ns`（`0.9149921`）。同 worker 的固定 V0.10 诊断亦为
  Native `7308918 / 7813684 ns`（`0.9353997`）、Clang `7336346 / 7918949 ns`
  （`0.9264293`）。原始失败和独立诊断同时保留；这些数据尚不证明具体频率/调度原因，
  也不授权把 raw 97% 门禁改成 optional。
- ARM artifact `9726583739` zip SHA-256 为
  `7ac4d11a79c0fa6731ec0d062ae32784f6118e1a401d5b56a6bed67192636692`，候选 report 为
  `bf322d0afbdb2b845454908d6eb7f76a5776320c44f4c0ccc70beeed7075e00c`，job log 为
  `e85cfab4b541fd412e2a888a11982aee4146ac520ee5c2dd1a54a3dd267eeba6`。该 artifact 中的
  机器码对照只覆盖 integer kernel，不能据此声称 proof-loop 机器码已排除差异。

## I21：Windows SDK 宏污染 LLVM bridge（实现修复待验）

- run `33288920505` 的 x64 Windows job `99196977330` 在 release/oracle prefix 均
  完成后，MSVC 14.51 编译 bridge 报 C2589。原日志 SHA-256 为
  `eecbbfc177045033fab0261ccf6d7aa8e23e1d261af11e7fd590e1748c96d4b3`。
- 逐项对应源码：`windows.h` 的 `min/max` 扩展破坏 `std::min/max`；
  `IMAGE_FILE_DLL`、`IMAGE_FILE_EXECUTABLE_IMAGE` 宏扩展破坏同名 LLVM COFF 枚举。
  用仅模拟这些 SDK 宏和三个 process-symbol 声明的 header，宿主 C++ 编译器对真实
  `ckc_llvm.cpp` 的 COFF 分支重现相同七处语法错误。这是宏污染复现，不代替 Windows ABI 验收。
- 修复计划：先把上述真实 translation-unit 复现加入 Unix Native regression，分别
  覆盖正常 `NOMINMAX` 和模拟既有 min/max 宏；再只在 Windows include 边界定义
  `NOMINMAX`、清除 min/max 和两个冲突的 COFF 宏。保留真正的 Windows SDK 函数声明、
  LLVM typed enum、所有链接/ABI/W^X 检查；不改 compiler flags 或常量值绕开检查。
- 回归先 red 后 green，随后完整本地 default/all-feature/release/Clippy 验证与六 host
  同一最终 SHA 的真实 SDK/MSVC 构建。该修复属于既有阶段 11 host build 契约，
  不修改语言、ABI、平台支持范围或门槛；自审无设计阻断。

### I21 本地实现与复审（真实 Windows CI 待验）

- 旧 run `33288920505` 的 ARM Windows job `99196977381` 现已结束，仍是修复前
  `c91c2c0` 的相同 SDK 宏展开错误；不是 `8c810ae` 的新失败。release 与 oracle cache
  均已成功保存，完整 log SHA-256 为
  `70128bd956e68d011b8c98faa67a946dc6123e8a70d6cf6baa51b35de44180df`。

- 文档提交 `800172c` 后新增真实 bridge translation-unit regression，先观察到与
  MSVC 日志逐项相同的七处宏展开错误；修复 Windows include 边界后，两种宏输入均通过。
  只定义 `NOMINMAX` 并清除四个冲突宏，不修改 SDK 声明、枚举值、导出表或 ABI。
  red / green 日志 SHA-256 为
  `b7fb73c865af66b7901b05df6c92f6a82c08d774a02bf76c91db873b47e64f68` /
  `c5cceb545398bfbafc610b75f1eecee6af2e12cc8e8efcbea2d78353c84b2276`。
- 默认 459、全特性 582（Native 94）、release 单元 53、release IR 58、all-feature
  all-target Clippy、fmt/diff 均通过。default / all-feature / release 日志 SHA-256：
  `ff44c58ace5005e27a60064eb96ec99444aacbf77c6a118561aa7fc5e7149c0d`、
  `d69e85972d983c071a8d662f47345a1476042e61d1566c358e0c255c7dc6c59a`、
  `1241024e5185384b48b656155af8ea6958ce743e36635e9bcdfb6617e478d2c1`。
  自审未发现宏越界到非 Windows 路径或测试替代真实 SDK 的情况；I21 仍待两架构
  Windows 以最终 SHA 完成真实构建与 Native 验收，不把本机模拟当作远程通过。

## I22：Windows static prefix 含默认开启的 LLVM-C.dll（本地修复通过，真实 Windows 待验）

- 新 run `33302144688` / `ae7a130` 的 Windows x64 job `99232169083` 恢复旧 v3
  cache 后，在 `validate-llvm-prefix.ps1:68` 以 `shared LLVM library in static prefix:
  LLVM-C.dll` 失败，尚未进入 bridge 编译。完整原日志 SHA-256 为
  `5e461c4ae76dd03709030d6594915966c572d8ea1d2349fae39f034ef9e99650`。
- 复诊固定 LLVM 22.1.8 源码：`llvm/CMakeLists.txt:894` 在 MSVC 下默认
  `LLVM_BUILD_LLVM_C_DYLIB=ON`；`llvm/tools/llvm-shlib/CMakeLists.txt:148–205` 的
  独立分支构建 `LLVM-C SHARED`。仅设置 `BUILD_SHARED_LIBS=OFF`、
  `LLVM_BUILD_LLVM_DYLIB=OFF`、`LLVM_LINK_LLVM_DYLIB=OFF` 不会关闭这个 C API DLL。
- `bootstrap-llvm.ps1` 安装后的旧断言只遍历 `lib/LLVM*.dll`，漏掉 Windows 的
  `bin/LLVM-C.dll`。旧 `c91c2c0` 尚无独立 cache verifier，因此这份有问题的 prefix
  得以保存；当前 verifier 检查 bin/lib 后拒绝它是正确行为，不是待放宽的误报。
- 修复计划：先对实际 bootstrap guard 做 bin/lib 模拟布局行为测试，并对 CMake
  配置添加显式 OFF 的 red regression；随后仅加入
  `-DLLVM_BUILD_LLVM_C_DYLIB=OFF`，让安装后 guard 同时检查 bin/lib。扩充独立 cache
  verifier 的 DLL 注入反例，确认仍拒绝污染。禁止事后删除 DLL 来伪造验证通过。
  同步固定 native build manifest 的 `build_llvm_c_dylib=false`；非 MSVC 原默认已为 OFF，
  不更改 Unix 构建路径或 LLVM 版本。
- 两个 bootstrap recipe 仍参与既有全量 cache identity，因此修复自然生成新缓存键；
  不复用不合格 Windows prefix，不修改/删除旧缓存，不缩减源校验或 static/CRT/ABI 门。
  默认/all-feature/Clippy 与真实两架构 Windows、同一最终 SHA 全十 job 必须重新通过。
  该项修复既有静态工具链契约，不改变语言、ABI、性能门槛或源 baseline；行内自审无设计阻断。

### I22 实现与行内复审

- 在计划提交 `41f11a7` 后观察真实 red：原配置没有独立 C API DLL 的 OFF 参数，
  原安装后 guard 接受 bin 中的 DLL。随后 manifest 字段的单独 regression 也先失败；
  修复后两个安装测试和原独立 cache verifier 的 bin/lib DLL 注入反例均通过。
  两份 red 日志摘要分别为
  `a694723a8f46551e69acf414f3f1010ea81584dfa2fc65b148d8b6ecca9e3740`、
  `056a2876405ffcfebd71eaa882d067f6be6328dfea132ddefec91a29835fe2ed`；
  针对性 green 日志为 `33cb838462f617df70c7143afd91c8d6f0bd856632debed1f5911c8e314ea035`。
- 默认 470、全特性 593（Native 94）、release 单元 53、release IR 58 全通过，
  0 failed / 0 ignored；all-target/all-feature Clippy、fmt、diff 检查通过。
  default / all-feature / release 日志摘要分别为
  `0858ec657f02367a01229a3ee4383b727005a4602d50a1dd224a1ae96f6dc3af`、
  `1cf438720d29bdb33a0cd60018baa2125ec1cb5636cf0775553c89417e0f59cf`、
  `c6b597fca783fb20c63647194a46a8ef8d35be157f95efcacee62800bf697272`。
- 自审确认仅显式关闭 MSVC 独立 DLL 构建、补齐安装布局断言及 manifest；没有删除
  已安装文件、放松 cache verifier、改变 LLVM 版本/ABI/门槛。新测试执行实际安装 guard，
  但不声称模拟目录替代真实 MSVC 构建。bootstrap/manifest 属于原缓存 identity，所有
  host 的新键须重新构建并验证，不绕过新键来复用旧 prefix。
- 同轮 ARM Windows job `99232168996` 也在 cache validation 的同一 DLL 上失败；日志
  摘要为 `fcc795342a98b44eee4581ca19d7650192fc1886203eb805544537fbe295e543`。
  I21 与 I22 仍必须由下一次完整矩阵的真实 Windows 编译和测试签收。

## I23：Unix run 登记子进程前丢失 SIGINT（针对性修复通过，全量待验）

- 当前 `33302635528/99233477608` 的 Native suite 卡在 public interrupt forwarding test；
  live UI 明确报告该 test 超过 60 秒。原 test 观察 OS child 存在即向 parent 发送一次
  SIGINT，随后无限 wait；不能把这个停滞误认为 LLVM 冷构建。
- 对 `5895242` 的原 Unix 模块做隔离真实信号复现：install/spawn/SIGINT/set_child
  顺序使 child 存活超过 2 秒，受控清理后 exit 1；install/spawn/set_child/SIGINT 则
  signal=2 退出。red SHA 为
  `179ada2373e1b547ca2f284247e4f047e7caa97e4a3f38bff0442d27a3808e41`。
- 根因是 handler 安装早于 PID 登记，原 `CHILD == 0` 分支直接丢弃中断；OS child
  可见不保证父进程已完成登记。复现确认产品漏洞，远端归因仍是与现象相符的推断，
  没有声称已读取远端进程栈。
- 按 `../implementation/11-interrupt-handoff-plan.md` 先提交计划，再以真实源模块和
  隔离进程做 red/green：一个原子表示 unarmed/pending/PID，登记负责交接 pending。
  给原 public test 增加明确失败期限及自身进程清理，不重发中断、不忽略测试。
  保留 245/CKR0006、Windows 行为、性能原门槛和同 SHA 十项 CI。行内自审无设计阻断。

### I23 针对性实现与行内复审

- 先提交 `cd11dda` 计划，再原样分离 Unix 模块，让 Native regression 与 CLI 编译
  同一份生产源文件。四个真实信号测试各运行于独立进程，不修改主 test runner 的 handler。
  首次 red 为两个登记前 case 失败、两个对照通过；red 摘要
  `9af162906596be60f55410f7af1bbcd9776b17ad44fc7486769a8f7e5a7189f7`。
- 最小实现只将 `0/pending/PID` 编码在既有 AtomicI32：handler CAS 未登记值到 pending，
  登记 swap 若收到 pending 则转发。四个 targeted case 全通过；green 摘要
  `d0eb141100af2e4d6688f0d14498b1f61329c641d3a8b4ec54befbc096391012`。
- 完整 `run::` 10 项通过，0 failed/ignored，包含原单次 SIGINT/245/CKR0006、stdio、
  checked failure、private protocol 与 JIT audit。新 timeout 反例同时证明超时返回错误、
  自有 child 不再存活且已经回收。该次日志摘要
  `2623ebd11be1044c59dbecf300a50dd36b2922f595d85f8275a17feabf9ca4b5`。
- 复审两种原子先后次序：handler 先成功则登记负责转发；登记先完成则 handler 得到正
  PID 后转发；已有 pending 不覆盖为 0。没有锁、分配、自旋、生产测试开关或第二个
  pending 原子。Windows 模块未变。原 public test 仍观察 child 存在后只发一次 SIGINT，
  不增加延时重试；期限只将无限等待变为明确失败并回收测试专属 group。
- 完整本地 default 470、all-feature 598（Native 99）、release lib 53 / IR 58、
  generated 3、mutation 10、fact audit 7、release verifier-cache 5 与 docs 16 均通过，
  0 failed/ignored；all-target/all-feature Clippy、fmt、release build、artifact/JIT audits
  和 version/licenses 通过。Native ABI=1、Runtime ABI=2、LLVM=22.1.8，manifest 仍为
  `b8b790dcfdd9652b1634d8d50075b1037298ec7cbcf3e7a5fefabb55d1f84874`。
- default / all-feature / Clippy 日志摘要分别为
  `f0f6409be2b19ac74d0a11527d7d2145a7e5e0702ff2a3612e5bb110ff749213`、
  `a849635bf570f641c6f2fa4536557aa4fedf121067e25d96a3fe3fd0b5d7e462`、
  `737935c4043b91f19c0c0ffa380c1a7a5ecb841ffc8aabab8f086d348aa165f3`；
  release lib / IR 摘要为
  `6615ab13b90d9598fb349eeda7c0d80478c60da0e49417ece15ecb6de6677046`、
  `c181b4e2c5e7e85df14835aa9bbd55929759424d544d5fdf005254fad03c278d`。
  artifact / JIT audit 摘要为
  `8bb2cdecd74783759124358c0a904aebfe6e89eac627e88d5e6009722ec1dc06`、
  `cd3a8a27f34c0e4f796287ba51b619ee0ed22c114cac5879606d983dfaac35a2`。
- 这不是阶段 11 签收；仍需该实现提交的首次性能及新同 SHA 六 host/十项 CI。

## I24：阶段 02 的三项 legacy preservation 回归未迁移（本地修复通过）

- 对当前 default/all-feature/release 完整日志逐项映射阶段 01–10 的测试命令，只有
  阶段 02 的 `optimizer_should_preserve` 匹配 0 项。实际运行原命令确认 0 passed /
  100 filtered out，不能当作三项验收成功；原日志 SHA-256 为
  `858bc97c0684f19e0952ccc09232b7da25759a1330fcd3132bd42cc6a5c742a2`。
- `9427203:tests/optimizer/passes.rs` 的原三项分别检查 O0–O3 break/continue CFG、
  PrintI32/PrintBool/PrintNewline 的数量与顺序、checked-bounds slice 内部调用和返回。
  阶段 11 删除旧 MIR optimizer 时没有把这三个完整断言组一起迁到当前测试 driver；
  现有零散测试覆盖相关行为，但不能替代这个明确的全级别回归入口。
- 修复计划（测试限定）：先提交本项复诊；新增 `tests/optimizer/preservation.rs` 并在
  `tests/optimizer.rs` 注册，复用 `support::compiler::optimized_module` 的 verified
  KIR 路径，保留三个 `optimizer_should_preserve_*` 名称及所有四个优化级别。
  控制流验证导出/return/有效 KIR；打印直接比较 typed runtime intrinsic 与 effect order；
  slice 保留类型检查，并在 checked bounds 下用 C 后端实际验证返回的 data/len，
  覆盖空和非空长度。允许设计规定的合法 inline，不把旧 MIR 的文本形式强加给 KIR。
- 执行原验收命令并严格要求 3 passed / 0 failed / 0 ignored，再执行完整
  default/all-feature、Clippy、fmt/diff 与文档回归。不得改成空过滤器、忽略测试或恢复
  旧优化路径。无需生产实现改动；若新测试暴露真实实现错误，先另行复诊再修复。
- 同步阶段 02 记录测试入口重新接通；阶段 11 同 SHA 全矩阵与总验收仍是必需条件，
  本项不改变原性能报告、数值门槛、语言/ABI 或 final acceptance 顺序。

### I24 实现与行内复审

- 计划提交 `c5652e0` 先于新增测试。原过滤器现在实际执行三项，0 failed / 0 ignored；
  日志 SHA-256 为
  `66bccd6880f982d7e54cd19b56d1b233b4db8ea9cbc8183ad25c1aa31066e6f6`。
- 逐条对照 `9427203` 的原三个测试：保留相同控制流输入、三个 typed print intrinsic
  和全部四个优化级别；使用已有 verified KIR helper，不重建旧 MIR optimizer。
  slice 的内部参数/返回类型仍被检查，O0/O1 要求定义保留；O2/O3 允许合法 inline，
  但实际 C harness 对每一级验证长度 0/1/4 及 null+0，返回 data/len 必须完全保持。
- 本轮只修改 `tests/optimizer.rs` 和新增的测试模块。与生产实现 `99ffb34` 比较，
  src/native/scripts/workflow/benches/Cargo 输入均未变化；不增加 release 依赖。
  C harness 复用仓库既有开发 Clang 调用方式，每次使用唯一 ignored target 子目录。
- 完整 default 473 / all-feature 601（其中 Native 99）均 0 failed / 0 ignored；
  default/all-feature Clippy、fmt、diff 检查通过，contracts driver 的 docs 16 项通过。
  default / all-feature 日志 SHA-256 分别为
  `ff3b0f5b4e32029bc3cccdd964ab6c12676214bc93cd9d54ee460312bd3f5533`、
  `2702545bf86ae3e5c678736d13b0999ce85c77916cf495dc509ae9f1f40e7d95`；
  两个 Clippy 日志为
  `e666c549efaa8beb959cf6122a4964b1ec3d617f1f3952b71d2d0ef8600ce2a8`、
  `17534c054d0782419494d5109b227e77724bb20a762df127ba870fffa9a3d749`。
- 单独文档命令最初误用不存在的 `--test docs` driver，Cargo 明确拒绝，原错误日志保留；
  改用仓库既有 `--test contracts docs::` 后实际 16 passed，不改任何测试或文档门槛。
  正确命令日志 SHA-256 为
  `316ce70b005b6123b956c7683e2c26ce5a13f385a50ef8d3f26eaec12b3e3d2d`。
- 复审未发现此测试迁移的新阻断项。此结论只关闭本地 I24 覆盖缺口，不替代尚未完成的
  同一候选 SHA 全十项 CI、阶段 11 与总验收。

## I25：Windows LLVM/bridge/Rust CRT 不一致且 COFF closure 缺失（本地修复通过，矩阵待验）

- `33302635528/99233477598` 的 bootstrap 在 13:25Z 完成，但随后 fact-audit 的 Cargo
  链接失败，测试尚未运行。完整 job log 的 SHA-256 为
  `e9788ec1be76ba5a448fac6e01df8224c0f27d76a7ccf6390355ecc2a398d729`；上传原日志为
  `fee21273c39395f6b0dc3d3a3c4ee15c0fc0917b08fdb51e11578b73558bbc68`。
- 两个 CMake configure 都明确警告 `LLVM_USE_CRT_RELEASE` 未使用。LLVM archives
  的 `MD_DynamicRelease` 与 bridge `MT_StaticRelease` 发生 LNK2038，大量动态/静态
  C++ runtime 重复符号；Rust 同次链接也默认 msvcrt。pinned LLVM 22 文档要求
  `CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded`，原字符串治理断言没有验证实际行为。
- 六个独立 LNK2019 来自 COFF driver 的 LibDriver/WindowsManifest。核对 pinned
  COFF/Common CMakeLists 并用真实 llvm-config 比较依赖集合，额外项仅为已有 DTLTO
  和缺失的 LibDriver/WindowsManifest；不是 CRT 错误的级联误报。
- 旧 verifier 允许这些 cache 入库，不能把“静态 archive 文件存在/没有 LLVM DLL”
  等同于“静态 CRT 内容”。本项将用实际 CMake commands、真正 COFF directives、
  Rust target-feature 和最终发布依赖审计闭环；旧缓存保留为证据但不得复用。
- 详细 red/green、文件边界、双语契约、同 SHA 全矩阵见
  `../implementation/11-windows-static-link-plan.md`。计划先提交，完全行内；没有
  `/NODEFAULTLIB`/强制链接/动态 CRT fallback，没有改变语言/ABI 或任何数值验收门槛。
- 测试准备首次在 ARM-only pinned Clang 请求 x64 COFF 被明确拒绝，不计为行为 red；
  原计划因此修正为每个 host 使用自身已编译 backend、完整矩阵共同覆盖两架构，
  保留 host-only 编译器与实际两 Windows MSVC 验收，不改 LLVM targets 或测试门槛。

### I25 实现与行内复审

- `c17e1bf` 原计划、`45a88da` host-only fixture 修订均先于生产修复。初次不支持的
  cross-target fixture 日志保留，不计为 CRT red。按 host backend 调整后，旧 verifier
  确实接受了真实 MD archive 加 static manifest；行为 red SHA-256 为
  `0afa88ab87a2eb9691811a7b591b0cf677a7e656afd3793312c249d87fcf460f`。
  配置 red SHA 为 `6ceadc75c30e1f197846f4822d8f072aaf272dfecd3e74ea4101ca075243c29c`。
- 新 guard 在 CMake build 前检查 C/C++ 实际 `/MT`，在安装及 cache boundary 用
  pinned llvm-readobj 检查每个 archive 的真实 RuntimeLibrary/DEFAULTLIB；拒绝
  dynamic/debug/mixed、无静态证据和工具/文件失败，纯汇编 member 不被强迫携带 CRT。
  逐 archive 调用避免 Windows 参数长度限制，不以 manifest 声明代替内容证明。
- 五个原 LLVM logical components 不变；COFF 另加 libdriver/windowsmanifest 的
  libnames/system-libs 同集查询与失败检查。DTLTO/LLD 顺序保留，未改成 link-all。
  producer 与 cache 复用同一 guard，完整 cache key 增加两项 verifier 输入，不删除
  原输入或提供旧键 fallback。新 recipe 的真实 Windows 重建仍是待跑远程验收。
- Cargo 对两个精确 MSVC target 的所有 profile 默认 crt-static；build.rs 在 bridge
  编译前拒绝错误 manifest CRT、缺失 target feature 或 COFF 组件。Rust 1.90 的两架构
  `--print cfg -C target-feature=+crt-static` 均实际报告该 feature。未更改桥接为 MD，
  未添加强制链接或忽略 default library 参数；Unix codegen/flags/ABI 不变。
- 实际 COFF 3 项回归通过：static/defaultlib-only 正例，dynamic/debug/单项 mismatch/
  mixed、损坏/缺失/空 archive、伪静态 manifest 和缺少 COFF component 反例全部闭环。
  green 日志 SHA-256 为
  `89c74da600be81b5b069809f997dd0dc42cf2c982c8b2faa3e262cf711c204ca`。
  原 default hash/path fixture 明确使用 double 且保留原断言，不承担 CRT 内容证明。
- 完整 default 475、all-feature 606（Native 102），0 failed / 0 ignored；default/all
  Clippy、fmt/diff、release lib 53 / IR 58 / Native build 全通过。
  default/all 日志摘要为
  `6ef13fc1481f2988873cff85207a5289e20b0994dc79e4ce72f4e3107398f3e1` /
  `1b25ae4bb075f7b1f7c8cd2eb2417611861f2240d6e57bcf3f93eabe7fffa696`；
  default/all Clippy 摘要为
  `605168ee5f2ed28c6012c5635383a30f2e3bb8b34076311d2baf4083427b71ee` /
  `56514c1a2cc5f211bb442aefd26a06483e44680b36e23262ff3e3e1a8e6dddfa`。
- generated 3、mutation 10、fact audit 7、release verifier-cache 5、docs 16 均通过。
  实际 release compiler 先 hardened ad-hoc 签名，再通过依赖审计；artifact/JIT audit、
  version/licenses 和原 Unix prefix 验证通过。compiler/JIT audit 摘要为
  `33076a392f155fc87e1b47c292d73786798f68ac06e4be64004c54abdd1a3b5f` /
  `a68309e4e044db6d3db7bf86cc7e93a67248bb867d6d30c0867b616d9a510ebf`。
- 当前本地 Native ABI=1、Runtime ABI=2、LLVM=22.1.8、Unix manifest SHA 仍为
  `b8b790dcfdd9652b1634d8d50075b1037298ec7cbcf3e7a5fefabb55d1f84874`。
  本地新测试只证明本机 backend 生成的 COFF 内容检查；不能冒充实际 MSVC 全链链接。
  后续 `d424270` 的首次性能失败、非代码环境复诊与唯一一次同 SHA qualification
  通过记录在 `../implementation/11-release-candidate-acceptance.md`，两轮原件均保留。
  两架构 Windows 和同 SHA 十项 CI 尚待完成，阶段 11 不签收。
- 新 `33316188869` dispatch 后，旧运行按既有 concurrency 终止为 cancelled：
  七项通过、Windows x64 失败、Windows ARM 与 Darwin x64 取消。已下载两项取消后
  完整日志，SHA-256 分别为 `a5b52581de28ae978edb42c1527ded549676777e071df2cdd3a91658db3bf4fd`
  与 `5b7e2ad5a3a0067e5620609c1e51f6db00f985ecd019c834357bc225abdf894c`。
  ARM 两次 configure 同样忽略旧 CRT 选项，在 oracle build 时被取消，没有 Native
  通过证据。新 x64 bootstrap 已采用 `452e16daafeb9644` recipe 前缀；这只证明开始
  正确配方的冷构建，不能代替后续 guard、链接和全部验收结果。
- 随后从新运行的实时原生日志核实，两架构 release profile 的实际编译命令 guard
  通过：x64 2478 个、ARM64 2471 个 C/C++ 文件全部 `/MT`。原始日志行链接见阶段 11
  acceptance；archive directives、oracle profile、真实 MSVC 链接及 suite 仍待完成。
- 同一 `d424270` 运行现有 8/10 必跑项通过：quality、双架构 performance、native integration、Linux
  ARM/x64 host、Darwin ARM host。AArch64 原 checker 的四组汇总为 `0.9996/1.0016`、`1.0001/0.9999`、
  `0.9997`、`1.3497`；下载原件 24 个 measurement 加 8 个 replay 文件全部与报告
  bytes/SHA-256 一致。Linux ARM 的 fact audit 7、Native 101、CLI 22 及三个发布审计
  全通过。Darwin ARM 的 fact audit 7、Native 102、CLI 22、hardened 签名及三项
  发布审计也通过，新 cache 两键保存成功。并发同键 cache warning 由 performance
  job 成功保存相同 release/oracle key 闭环；不是跳过验证。Linux x64 的 fact audit
  7、Native 101、CLI 22、三个发布审计及两条 cache save 同样通过。x86-64 性能
  四组汇总为 `1.0516/0.9983`、`1.0184/1.0078`、`0.9944`、`1.5132`，32 个
  实际文件与报告相符；integration 605 项、artifact fixture 5 项和 sanitizer 8 项
  均通过。Darwin x64 的 Native 102 项含全部 I23 handoff/public SIGINT 回归，CLI、
  签名及三个发布审计也通过。其余两架构 Windows 未终态，不能
  签收 I25 或阶段 11。

## I26：Windows x64 首次进入 Native suite 后暴露 COFF 输出与 JIT image-base 缺口

- 新 run `33316188869`、SHA
  `d4242700489e632cd3ef2d2d9c88610b683f1fbf` 的 Windows x64 job
  `99269971157` 完成新 recipe 的 release/oracle bootstrap、pre-LLVM fact audit 及
  artifact 上传，随后 `Run required native suite` 为 62 passed / 30 failed。完整日志
  SHA-256 为
  `2315bc4d21c60ea36ff12085864733a3879085102db34bdfc5086602ff89f0ba`；上传
  artifact ID `9737051325`，zip / 原 fact-audit 文件 SHA-256 分别为
  `bee37c361e63f616374a215f95a008e321d1e33756e0a10e40e2d2d4b90aab8f` /
  `27c2a74b0ed7af65bfea3706d849ac3bf01725a1e5f6ebe2ce8a8ecf289d780b`。
- 十个 artifact/shared/executable/sanitizer/differential 失败都由同一命令构造错误触发：
  COFF driver 已是 `lld-link`，公共尾部却传 `-o <output>`；driver 忽略 `-o` 后尝试把
  output 当输入。修复必须是 COFF `/out:<output>`，不能接受 warning、预创建空输出或换外部 linker。
- cache/run/JIT 的级联失败都收敛为 `Symbols not found: [ __ImageBase ]`。固定 MSVC
  runtime C objects 带 x64 `.pdata` image-relative relocation；CK 的自定义 JITLink
  layer没有 COFFPlatform header，且正确地禁止任意 process-symbol generator。修复使用
  manifest/hash/cache 绑定的 x64-only JIT anchor，并保持 anchor、五 runtime objects 与
  program object 在同一固定 reservation/JITDylib；不开放 host symbol、不切回
  RuntimeDyld、不删除 unwind sections。
- `checked_native_thunks...` 与 `checked_calls...` 不是 ABI 产物错误：日志中的 definitions
  正确带 `dllexport`，旧断言只匹配 `define i32`。修订后仍验证 definition 行、i32 status、
  result pointer/void 规则及 internal implementation，不把测试移除或按 Windows skip。
- 详细 docs-first/TDD/远程计划见 `../implementation/11-windows-static-link-plan.md`
  Task 5。当前 I26/I25/阶段 11 均未签收；同 run 的八项 success 与后续提交不得聚合。

## I27：Windows ARM64 自定义 RuntimeDyld creator 漏配 COFF 符号责任

- run `33316188869`、SHA
  `d4242700489e632cd3ef2d2d9c88610b683f1fbf` 的 Windows ARM64 job
  `99269971150` 自然完成为 failure：release/oracle bootstrap、静态 CRT/archive 检查和
  fact audit 7/7 通过，Native suite 出现 18 个明确失败标记后以 `0x80000003` 终止。
  完整日志 SHA-256 为
  `0e9351c157354ea90a4cb8908d5ac524875966abc6a351bc92630162263ab67f`；fact artifact
  ID `9737795689`，zip / 原文件摘要为
  `2752149aea74bb5ecde01b6823437e89ee334e457e99ce5674b44bc0d3024c78` /
  `1316726ad12ae778e9e5ecaa5c4cb58b073539dbd4a861f7cf42b0cc478f8250`。
- I26 已覆盖该日志的 `/out:` 与 `dllexport` 类失败；其余 cache/run/JIT 路径在子进程
  七次、父进程最终两次触发 pinned LLVM `Core.cpp:2803`：
  `Resolving symbol with incorrect flags`。x64 JIT anchor 对 ARM64 不适用，不能扩大范围。
- 复诊 pinned LLVM 22.1.8 后确认：CK 的 ARM64 COFF 分支为安装 audited section memory
  manager 覆盖了 LLJIT 默认 object-layer creator，但只构造并返回
  `RTDyldObjectLinkingLayer`。官方 LLJIT 的 COFF creator 还会设置
  `setOverrideObjectFlagsWithResponsibilityFlags(true)` 与
  `setAutoClaimResponsibilityForObjectSymbols(true)`；前者处理 resolved/declaration flag
  一致性，后者处理 COFF weak/COMDAT 等 object symbols 的责任认领，恰与断言路径闭环。
- 修订见 `../implementation/11-windows-static-link-plan.md` Task 6：先用局部 contract
  保留 red，再只在既有 ARM64 COFF layer 上恢复两个官方设置。audited memory manager、
  process search disabled、W^X、allowlist、backend identity 和全部 JIT tests 保持不变；
  不关断言、不跳过、不换未经签收的 JITLink。
- I27 是新增真实阻断，必须由修复 SHA 的本地完整门、schema-6 性能和同 SHA 十项 CI
  签收。旧 run 的八项 success、x64/ARM64 失败与后续结果不得拼接；I27/I26/I25/阶段 11
  当前都保持未通过。

### I27 本地实现与行内复审（性能/远程待验）

- docs-first 提交 `cdbe498` 只含复诊、证据和计划，提交前 docs 16 / diff 通过。随后新增
  contract 在旧 bridge 上实际 1 failed / 0 passed，首个缺项为 typed `object_layer`；
  修复后 default 与 native-feature targeted 均为 1 passed。contract 先剥离行注释和空白，
  再把检查限制在 ARM64 COFF 分支及返回顺序内，注释或其他 creator 不能制造假阳性。
- bridge 只把原 bare constructor 拆成同一 `RTDyldObjectLinkingLayer` 实例，依次设置
  responsibility-flags override 和 object-symbol auto-claim 后上转型返回；audited allocator、
  ARM64 条件、x64/Unix else 分支和 process-symbol setup 均未改变。用实际 pinned prefix
  编译通过，不是只做文本测试。
- 本地 default 477 / all-feature 608（Native 102）、release lib 53 / IR 58、generated 3 /
  mutation 10 / fact audit 7 / verifier-cache 5 / docs 16 全部通过。首次 all-feature 漏传
  `CKC_CLANG_ORACLE` 时三项真实 COFF fixture 明确失败，补回冻结 oracle 后完整重跑通过，
  没有改成 skip。
- 两种 Clippy、fmt/diff、release Native build、Apple 的 Linux-only sanitizer capability
  记录、artifact/compiler/JIT audits、version/licenses 和 prefix manifest 验证均通过。
  英中 ABI 同步说明该设置只恢复官方 compatibility contract。原 schema-6 性能及同 SHA
  十项 CI 尚未执行，故 I27 仍未签收。
- 精确修复 SHA `7b03f76e1139ec91a5962ca18e696c2c127604c2` 的本机 schema-6 门随后通过：
  unchecked Clang / replay `1.0021 / 1.0008`，checked `0.9971 / 1.0033`，raw proof
  `1.0015`，optimizer suite median `1.1159`；report SHA-256 为
  `78f90b33afcebcbdcc01eb6ea15b77ad9f0b18c2dcfc640a84c3c8050bf34984`。
- 同 SHA run `33332458652` 当前 quality `99313407097` 与 Darwin ARM64 host
  `99313407133` success（2/10）。ARM host 原始日志 SHA-256
  `455a09811094e61afe66fddfc4268f4ab204490777da4883b6cdadaa1745834b`；fact artifact
  `9739282165` 的 zip / 原文件摘要为
  `07ecef3c1b3fb18856e757407f31408aaddd3cc4c75e1dfcf14a46fe708a775e` /
  `573062b89c1cee7a0d403ed7ff52c5fd6d08979441c97c83f391b7a72634aedf`。
  fact 7、Native 102、CLI 22、签名与三项发布审计、两条 cache save 均通过；其余八项
  当时仍运行。
- 同轮 AArch64 performance `99313407109` 随后 success，矩阵为 3/10。原 checker
  汇总为 unchecked `0.9978 / 1.0017`、checked `0.9997 / 1.0032`、proof `1.0020`、
  optimizer `1.3436`，所有 individual gates 通过。完整 job log、artifact zip、report、
  checker 摘要分别为
  `6b296445e68e9bad057a92409aea5266bc3cbbb03e97140614e672ebed9ad5cb`、
  `a73f48580aa72897ac0f1f0cdbf203472fb8fd2a18d943191a4fc419056b8564`、
  `2f250e1414242703a1c611d3c5e2e1544a0eb40b630cdf7c2f09e31ed3cbb2ae`、
  `6699fb110603d3bb5abadf42a7f570537eec88a635c997949eaa3a2a0277da4c`；artifact
  `9739404998` 中 24 个 measurement 与 8 个 replay 文件逐项验证 32/32 相符。其余七项
  当时仍运行。
- Linux ARM64 host `99313407129` 随后 success，矩阵为 4/10。fact 7、Native 101、
  CLI 22、compiler/artifact/JIT audits 均通过；完整日志 SHA-256 为
  `b12ea8aacba8a6013f436c95d71904863cf11445f54250dbe1eb1dc8fd34fc96`。fact artifact
  `9739439013` 的 zip / 原文件摘要为
  `c5da52139290e5870f22b048e6c0cba533b15e32f720ca36ef3cbaa568e8fc2a` /
  `9f23d5fff246ade56df8fe39e0528e273ca9a21789649f097a235d0ed1af42fe`。该 host 的
  release/oracle cache save 只因同键竞争警告；同一 AArch64 performance job 的原日志
  证明两个精确相同 keys 均已成功保存，因此不构成未验证 fallback。其余六项仍运行，
  继续保持 I27 未签收。
- native integration `99313407031` 随后 success：完整 all-feature suite 607 passed / 0
  failed / 0 ignored，另有 artifact fixture 5/5、sanitized ownership 8/8 及 artifact/JIT
  审计通过。完整日志 SHA-256 为
  `48227f664b063766d598dfe6a58e596117b4f737bb75f080626f94a26481c510`。
- x86-64 performance `99313407083` 随后 success，矩阵为 6/10。原 checker 的
  unchecked Clang / replay 为 `1.0496 / 1.0007`，checked 为 `1.0064 / 0.9979`，
  raw proof 为 `0.9944`，optimizer suite median 为 `1.5071`，全部 individual gates
  通过。job log / artifact zip / report / checker SHA-256 分别为
  `9a8adf924590fb128facf7ca4ab82d2a08b8383d4b5ffb6ab8c5bbcc1f1d8aa6`、
  `4196c3f4aaa6bab14c3ec70be0fe730dbbcc562bc0a53a63b8776041630b003e`、
  `c0a3d263791159745e4b85c0212d8028c8f6f2c9fc2bcedeb2bd03da4b1464a8`、
  `c194c136d15f7ea442b199deee1d69b6dc2b4f9c944ce505709daaa37d931ee8`；artifact
  `9740011519` 中 24 个 measurement 与 8 个 replay 文件逐项验证 32/32 相符。其余
  四项仍运行，继续保持 I27 未签收。
- Linux x86-64 host `99313407167` 随后 success，矩阵为 7/10。fact 7、Native 101、
  CLI 22 及 compiler/artifact/JIT audits 均通过；完整日志 SHA-256 为
  `85e58ebbdd3213633200a37b9ae9289195c9bbf13eaf8a8a866380956566903a`。fact artifact
  `9740082468` 的 zip / 原文件摘要为
  `d41d06ec6e19d91d55b381883ac7a5fc8c94901881cf6478d854865a6cb0bdc7` /
  `db2b31037c193d73ee072558f8e3f2f17d27b79dd457c6b5d18a567e15648558`。其余三项
  仍运行，继续保持 I27 未签收。
- Darwin x86-64 host `99313407110` 随后 success，矩阵为 8/10。fact 7、Native 102、
  CLI 22、release/oracle cache save、hardened ad-hoc 签名及 compiler/artifact/JIT audits
  均通过；完整日志 SHA-256 为
  `a5d708d3efefd391f74df63a647f5a58afb172a8eec98c6a129b928726ab214a`。fact artifact
  `9740622726` 的 zip / 原文件摘要为
  `423cf5542f1013acb33e02c1b6a0ae418a3760adeecf55e2ed6c08f07e930c65` /
  `4dcaddecb5366d7227efa2f20d989084fe7d7f770e579af6d2981dbcf62fc20a`。其余两项
  Windows host 仍运行，继续保持 I27 未签收。

## I28：Windows x64 JIT support array reference 窄化局部 object 容器

- run `33332458652`、SHA
  `7b03f76e1139ec91a5962ca18e696c2c127604c2` 的 Windows x64 job
  `99313407116` 自然完成为 failure。release/oracle bootstrap、77 个 MSVC archive 静态
  CRT 检查、x64 JIT support object、prefix 验证与两条 cache save 均成功；首次执行
  fact-audit test 编译时以 E0277/E0308 停止，未进入 Native/CLI 或发布审计。
- 完整日志 SHA-256 为
  `5265e7791eef8994a24209daab993566d2f84d2f58ef40a26515604ed5b801a9`；失败 artifact
  ID `9741692125`，zip / 原文件摘要为
  `f262aa42e985645d8362cf92569fbb3009a853288f721155a0201012cb571c2d` /
  `3e8f01a73123facfcc3cfad3e977cf31153517144eccf187b23464178521be60`。
- 根因位于 `embedded_jit_objects`：无类型注解的 `Vec::with_capacity(6)` 在 Windows x64
  cfg 下先接收 `include_bytes!(CKC_RUNTIME_JIT_SUPPORT)` 的 `&[u8; 621]`，从而被推断为
  `Vec<&[u8; 621]>`；随后不能 `extend` 五个 `&[u8]` runtime objects，也不能满足声明的
  `Vec<&'static [u8]>` 返回类型。Unix 与 Windows ARM64 不编译该 x64-only push，不能
  覆盖此 production cfg。
- 修复边界见 `../implementation/11-windows-static-link-plan.md` Task 7：先锁定 production
  source red，再只给局部容器显式 `Vec<&'static [u8]>` 类型，让标准 unsize coercion 在
  push 边界发生。不得复制 bytes、改变 anchor-first + five-runtime 顺序/数量、移除 cfg，
  或改动 bootstrap/cache、ABI、ORC、W^X 与任何门槛。
- I28 是新增真实阻断。原 run 的八项 success 与后续结果不得拼接；仍在运行的 Windows
  ARM64 job 必须自然结束并独立归档。修复后必须由完整本地、schema-6 性能和新 SHA 的
  十项 CI 同时签收 I28/I27/I26/I25/阶段 11。

### I28 本地复诊、修复与资格复验（远程待验）

- docs-first `bc009e5` 后，production-source contract 在旧实现真实 0/1 red；只给局部
  `objects` 增加 `Vec<&'static [u8]>` 注解后 1/1 green。行内复审确认 cfg、anchor-first、
  五 runtime objects、容量、bytes、顺序和零拷贝语义均未改变；没有把支持对象送入 AOT
  artifacts，也没有触及 ABI、ORC、process symbol isolation 或 W^X。
- default 477 / all-feature 608（Native 102）、release lib 53 / IR 58、全部独立小门、两种
  Clippy、fmt/diff、artifact 5 与发布审计均通过。旧 Unix prefix 的 manifest 缺少实际
  `LLVMDTLTO` 条目，被 verifier fail-closed 拒绝；按当前 recipe 全量重建的 release/oracle
  manifest 摘要为 `8a0d25cdcd729cd35be139d9f3b571d3a0769a380d1fce1e9731292119dc290c` /
  `b073daad34f4dfd5055614c7893b42c38f875cb54198b528729247dd3d13f934`，双 profile、实际
  compiler dependency、artifact 与 JIT audit 全部通过。没有手改旧 manifest 伪造成功。
- 首次 schema-6 原件被原 checker 拒绝：Dijkstra `3.7952x`，report SHA-256
  `0c4c5420d664028e8a0341a754f938aa45ff077b63f6a8f21a0b3efafa8d38bc`。当时共享主机刚完成
  4361-target LLVM 构建，CPU idle 多轮仅 23%–61%，并观测到多核 Node/index/FSEvents；
  六个 optimizer case 相对此前同机合格轮次都变慢，而四组同轮 runtime 对照仍约 1.0。
  这些是环境干扰证据，不把失败改写成通过，也不声称逐样本调度的直接因果。
- 既有资格规则满足后只追加一次完整同参数测量：连续六个当前 idle 样本 74%–84%，无
  高负载构建/索引进程；原 checker 对所有保留样本 exit 0。unchecked Clang/replay
  `0.9993 / 1.0002`、checked `1.0014 / 1.0011`、proof `0.9965`、optimizer suite
  `1.0929`、Dijkstra `1.9954x`。report/checker SHA-256 为
  `9618e947dd66a31aa0258691117087d3e040392fc9c4ed64710e3a0d5496a682` /
  `79181453144d5862641e70aa0a710fd27b928cc4562eac2f746c2424e83f8a0c`。首次失败与资格
  原件、24+8 artifacts、preflight 均独立保留；没有第三次计时或任何门槛修改。
- I28 本地阻断已消除，但真实 Windows x64 编译/执行和 Windows ARM64 I27 路径只能由
  新提交精确 SHA 的完整十项矩阵证明；旧 run 的八项 success 继续不得拼接。

## I29：Windows ARM64 host conformance 与 cache touch

- run `33332458652` 最终 completed/failure；Windows ARM64 job `99313407132` 自然完成
  release/oracle 新 recipe bootstrap、双 cache 保存/验证和 fact audit 7/7，Native suite
  为 87 passed / 5 failed / 0 ignored。完整日志 SHA-256
  `7b13024e4f5177674a999f03ce8bbe2cb438eece7b90c98480ffe5ee80b60720`；fact artifact
  `9742713855` 的 zip / 原文件摘要为
  `bc023be4906d0a3bedf39d3b6fd32ed1599c48a69869211602669dd73ffaea85` /
  `dee966674a5e187610d392b266a03f6a2746e930f3dc39ba13e26d564cc975b4`。
- 原 I27 incorrect-flags assertion 与 `0x80000003` 消失；JIT cache/run、完整 object graph、
  checked modes、W^X audit、standalone executable 和 generated differential 实际通过。
  五个剩余失败是新的 host conformance 阻断，不能用这些通过项抵消。
- ABI 失败来自正确的 Windows `dllexport` 与写死 `define [2 x i64]` 的断言不匹配；两项
  ownership 失败来自名为 macOS 的测试和通用测试都硬编码 JITLink。修订只能匹配合法
  storage class，并把两项都改为既有六 host policy 断言；不得用 cfg skip 把 Windows
  执行数从 92 降到 91，也不能改变 ARM64 RuntimeDyld 生产选择。
- differential 在第一个 C oracle `GetProcAddress("scalar")` 即失败：Windows Clang
  fixture 没把 MIR exports 传给 COFF linker。必须由同一 MIR export 集生成完整 `/export:`
  参数，并继续对 oracle/Native 双库实际逐符号执行；不能修改 CK Native exports 掩盖 oracle。
- cache warm-hit 是生产缺陷：Windows no-follow entry handle 只有 read access，无法完成
  `File::set_times` 所需的 `FILE_WRITE_ATTRIBUTES`，错误又按 best-effort 被忽略。最小修复
  只增加精确属性权限，保留 `GENERIC_READ`、`FILE_FLAG_OPEN_REPARSE_POINT`、owner-only
  root、entry bytes 与 cache failure 不影响程序执行的边界；禁止延长 25 ms 测试等待。
- 计划与验收见 `../implementation/11-windows-static-link-plan.md` Task 8。I29 修复必须
  叠加 I28 后由新的同 SHA 全十项矩阵证明；`7b03f76` 的八项 success 不得拼接。
- docs-first `fd594d9587cee8cf5c69d797fde0dab2940976c4` 后，cache production-source contract
  在旧实现上实际 0/1 red，精确 `GENERIC_READ | FILE_WRITE_ATTRIBUTES` access mask 后 1/1
  green；行内复审进一步锁定三个 access/flag 常量的精确值，不能仅凭名称假绿。没有
  generic write、entry rewrite 或 sleep 修订。ABI/ownership/oracle fixture 分别按合法
  storage class、冻结六 host policy、同 MIR 完整 exports 做最小修订。
- 完整本地非计时门已通过：default 478、all-feature 609、Native 102、release lib 53 / IR 58、
  generated 3 / mutation 10 / fact audit 7 / verifier-cache 5 / docs 16，均 0 failed/ignored；
  两种 Clippy、fmt/diff 与全部 release audits 也通过。
- 共享主机高负载期间没有启动计时；一个满足 idle 数值但末尾出现 project-index 的正式
  preflight 也被拒绝。随后六个样本 76%–85% idle、首尾无高负载编译/索引，只执行一次
  schema-6 benchmark。同一原始结果由完整身份环境下的原 checker 通过：unchecked
  `0.9998 / 0.9995`、checked `1.0055 / 1.0005`、proof `0.9976`、optimizer suite
  `1.1641`、Dijkstra `2.1876x`；24+8 artifacts 完整保留。两次 checker 缺身份环境的
  前置条件拒绝没有取新样本，不构成重跑或数值筛选。现在只剩新精确 SHA 十项远程矩阵，
  未用本地源码 contract 替代 Windows 实机执行。

## I30：Windows freestanding memory helpers 与 host executable path

- 精确候选 `f460c2b94f204738c2cbe6b4d9509409665a78ac` 的 run `33349902056` 自然结束为
  8/10：除 Windows x64 `99361072001`、Windows ARM64 `99361071997` 外全部 success，
  两项 schema-6 performance 也保持原门通过。x64 / ARM64 完整日志 SHA-256 分别为
  `b4bc5119df80b28c7df7ad98ba6c91fffcffba1cb537c14d952893f587f6dab6` /
  `2f189861ce8b7a3847fee3a91e75f65c1f2ba8d4daa988f11e3e3de37a5c8f0f`。
- ARM64 fact audit 7/7、Native 92/92；I29 五项实际 Windows 路径均已通过。CLI 21/22 的
  唯一失败发生在 executable build command success 之后：fixture 断言原始 base path
  存在，没有使用 production Windows `.exe` artifact path。修复只统一测试与
  `NativeArtifactPaths`；禁止改 CLI 命名、删除存在性断言或 cfg skip。
- x64 fact audit 7/7、Native 68/92；24 个失败共享 in-process LLD/JIT 的同一 undefined
  `memcpy`/`memset` 诊断，producer 为 `/O2 /Zl` 编译的 `format_float.obj`。MSVC 优化器
  可以为合法 C 生成这些调用，而五对象 freestanding closure 与唯一 `kernel32.lib` 不提供
  CRT helpers；这是生产闭包缺陷，不是测试可移植性问题。
- 最小修订在既有 Windows `platform.obj` 定义 byte-loop `memcpy`/`memset`，并对这两个
  MSVC definitions 局部 optimize off/on 以免重新识别为自调用。五对象 manifest、order、
  `/Zl`、无 CRT、allowlist、CK exports、cache identity 和 ABI 均保持；不得加第六对象或
  default library。
- 计划与验收见 `../implementation/11-windows-static-link-plan.md` Task 9。先做旧源码
  production-source red，再做最小 green；最终必须由新精确 SHA 的两架构 Windows
  7/7 fact、92/92 Native、22/22 CLI 及同 SHA 十项 success 证明，不能拼接本轮八项。

## I31：MSVC 拒绝定义已启用的 `memcpy`/`memset` intrinsic

- 精确候选 `991d192f13b845abc2e35e9406982093fe07b44e` 的 run `33351217336` 已自然结束为
  completed/failure、8/10。quality、native integration、Darwin/Linux 双架构 host 和两项
  performance success；Windows x64 `99364841264` 与 ARM64 `99364841227` failure，期间
  没有取消 jobs，也不把八项 success 与后续修复 SHA 拼接。
- x64 已完成 cold LLVM build，但真实 `cl.exe` 用 `/O2 /W3 /WX /GS- /Zl` 编译
  `native/runtime/windows/process.c` 时，对 `memcpy` 和 `memset` definitions 分别报告
  C2169。完整 job log SHA-256 为
  `60112035de5a469e3d28b1bc915f4e91986a871416fc7343667dee19624db022`；失败发生在
  Native suite 前，I30 的本地 `clang-cl` 对象检查不能替代该 red。
- 官方诊断说明 C2169 是定义已声明为 intrinsic 的函数；官方 `#pragma function` 会在源文件
  后续范围强制指定 intrinsic 生成函数调用。I30 的 optimize-off 只能阻止 helper loop 被
  重新优化，不能改变 intrinsic 身份。最小修复是在同一 `process.c`、optimize-off 之前增加
  唯一 `#pragma function(memcpy, memset)`，并继续保留 optimize off/on。
- 不改 `/O2 /Zl`、五对象 manifest/order、唯一 `kernel32.lib`、CRT-free 审计、CK exports、
  ABI、cache identity 或 CLI artifact path；不得用全 runtime `/Oi-` 或 default CRT 绕过。
  计划与验收见 Task 10/I31，先 source contract red→green，最后由新精确 SHA 的两架构真实
  MSVC 与同 SHA 十项矩阵签收。
- docs-first `53ef61e` 后，production-source contract 在旧源码真实 0/1 red，只增加唯一
  `#pragma function(memcpy, memset)` 后 1/1 green；现有 optimize off/on、两个 byte loops、
  五对象和 `/O2 /Zl` 断言全部保留。pinned `clang-cl` ARM64 COFF 实际定义两个符号且没有
  同名 undefined。完整本地非计时门保持 default 479 / all-feature 610 / Native 102 及原独立
  计数全绿；性能与真实 MSVC 修复 SHA 矩阵尚待签收，不能据本条关闭 I31。
- 旧 run 的 Windows ARM64 job `99364841227` 也已自然复现同一组 C2169，完整日志 SHA-256
  `18cea61e528bae68e562501d2dfd8592269b4f21cc12d8b2d8f1b774930c52c1`。它与 x64 一样
  未进入 Native suite，故修复后仍必须由两架构各自完整 bootstrap/执行门签收。
- 最后结束的 Darwin x64 `99364841169` 通过 fact 7、Native 102、CLI 22 与全部 release/
  artifact/JIT audits；原始日志 SHA-256 为
  `255ac924fd7df59fc03e468de1c9957b44f7325b4cd531d525f9ca271701fcab`，fact artifact
  `9747706336` 的上传 ZIP SHA-256 为
  `9b378f7881ce6f13d63a6aac3c99493a2fb5fb35e788b8cee51b4e5363024ff1`。因此旧 run 的
  自然终态完整，仍只证明 I31 red，不关闭修复后的真实 MSVC 门。
- 高负载期间两个 preflight 与持续监视均不启动 benchmark；连续资格曾在 4/6、5/6 被真实
  外部任务归零。最终六样本 `81.73 / 85.98 / 85.58 / 85.85 / 86.23 / 79.75% idle`
  且无高负载编译/索引/Node/Java/VM 后，只执行一次完整 schema-6。原 checker 与归档只读
  复验均通过：unchecked `1.0003 / 0.9979`、checked `1.0072 / 1.0056`、proof `1.0015`、
  optimizer `1.1068`、Dijkstra `2.0762x`；报告 SHA-256 `8a125478...`，24+8 artifacts
  完整保留，没有重跑择优或修改阈值。

## I32：COFF x64 `__ImageBase` 晚物化产生负 image-relative relocation

- 精确候选 `5fa94b089156ecae36a24c90d4c580fc473fbd83` 的 run `33364897799` 已有八项
  success；Windows x64 `99403408409` 自然 failure，ARM64 仍在运行且不会被取消。x64
  bootstrap 与 fact 7/7 已通过，确认 I31 的真实 MSVC C2169 已关闭到下一层执行门。
- Native 78/92；7 个 cache、4 个 JIT、3 个 run failure 的 stderr 全部是 runtime `.pdata`
  `Pointer32` relocation 到 `__ImageBase` 得到 `-0xcb0` 等负 image-relative 值。完整日志
  SHA-256 `8c3a22a7d14038230d9d760d1cced0383c82e45abdb2c1b563bfaf4b08ab8b75`；fact
  artifact `9755398106` 的 ZIP / 原文件 SHA-256 为
  `a95b9547cbc119b12cfe8a490af6c124c2afe7788d4fa7931d47119da17706bf` /
  `61920673061079661677c0abc0e8fb8974be26c57b813d179821a52a1b7dc5b9`。
- pinned `MapperJITLinkMemoryManager` 把首个 materialized graph 放在 reservation 起点，余量
  从其后向高地址分配。现实现虽先 add anchor，却在全部对象加入后按 symbol set lookup；
  `??_C...` 先触发 runtime materialization，anchor 后置于更高地址，故 image-relative 为负。
- 最小修订只让 COFF x64 anchor 单独 add 并 lookup 成功后再 add 余下固定闭包。继续使用
  JITLink/512 MiB reservation/audited mapper/W^X，禁用 process search；不改 anchor bytes、
  六对象 JIT、五对象 artifact、ARM64 RuntimeDyld、ABI、cache 或性能门槛。计划与验收见
  Task 11；先 production-source red→green，最终由新 SHA 的完整十项矩阵签收。
- docs-first `ae564c5` 后，production-source contract 在原实现真实 0/1 red；最小实现
  `9be13325e258a2cef2789ee82853ae18b5530c37` 的同一 contract 1/1 green，并额外通过
  显式激活 x64 新分支的 C++ syntax-only 编译。顺序、COFF/MSVC/Clang x64 guard、lookup
  fail-closed 与非 x64 原 loop 都由 contract 锁定；red / green 日志 SHA-256 为
  `1e1a0ef2a19223d116cd73579ec29a6ea7567872d8c7b9aa9de88e423d57706d` /
  `3894ce814560553a13e84e1ed92290fc75f3f5f0fa0c83cdf6b9b07ae97d2e72`。
- 最终测试版本的 default 479 / all-feature 610（Native 102）、两种 Clippy 与 fmt/diff
  全绿；相同生产实现的 release lib/IR、独立 Native 与全部小门、release/audits、双 prefix
  也通过。唯一合格 schema-6 报告 `8a125478...` 仅由原 checker 只读复验，没有重计时、
  rebaseline 或改门槛。旧 run ARM64 仍自然运行，新 SHA 十项尚未 dispatch，因此 I32
  继续保持阻断，不能把本地与旧八项 success 拼接成签收。

## I33：Windows release audit 隐式依赖未初始化的 `dumpbin` PATH

- run `33364897799` 的 Windows ARM64 job `99403408399` 自然结束为 failure，但先通过
  bootstrap、fact 7/7、Native 92/92、CLI 22/22 与 static-CRT release build；唯一失败在
  `scripts/audit-ckc-release.ps1:11` 的 `Get-Command dumpbin.exe`。完整日志 SHA-256
  `77066a288bb1db4f2b97ede1baf0a397479f334138a74629080dee4b9727ac97`；fact artifact
  `9757383387` 的 ZIP / 原文件 SHA-256 为
  `eab16b85762ec0f04682b5d08512e8dfee046efe5df11e75a91efe5daf017ebd` /
  `541fe8d17eda3ee101c80dc820011dea30f91a23934cad5fbbe5e0647b4b546c`。
- audit 在读取 candidate imports 前失败，所以不能诊断成动态 CRT；真正缺口是脚本假定 runner
  已初始化 Visual Studio developer PATH。release/oracle prefix 本轮均验证并保存，且已经包含
  pinned `llvm-readobj.exe`；用其绝对路径与 `--coff-imports` 可消除未冻结环境依赖。
- 修订必须在 missing tool、inspector nonzero 或 forbidden import 时继续 fail closed，并保留
  version/licenses、全部 forbidden runtime/compiler 名称与实际双 Windows remote audit。不得
  通过查找任意系统 `dumpbin`、放宽 regex、跳过步骤或改链接/ABI/cache 修复。计划见 Task 12。
- docs-first 提交 `91f3f1d` 后，production-source contract 在旧脚本真实 red；实现提交
  `bde2ed1421350d59a02034b56f7bb171b53c97e5` 改为从 `CKC_LLVM_PREFIX` 解析唯一
  `bin/llvm-readobj.exe`，regular-file、nonzero、forbidden import、version 与 licenses 均继续
  fail closed。targeted green、PowerShell parse、default 480、all-feature 611（Native 102）及
  全部本地阶段 11 门通过；原 schema-6 仅只读复验。I33 的产品/本地修复已闭环，但在两个
  Windows job 和同 SHA 十项 CI 全绿前仍保持阻断，不把 source contract 当作 PE 行为证据。

## I34：COFF x64 对远 process symbol 的直接 `PCRel32` 超出范围

- 精确 SHA `be4b77d` 的 run `33393261918` 最终 8/10；Windows x64 job
  `99491674256` 的 I32 negative image-relative/Pointer32 已消失，但 Native 仍为 78/92。
  14 项全部来自 `ckc-runtime-4.o` 对 `GetStdHandle`、`WriteFile`、`ExitProcess` 的直接
  `PCRel32` 无法跨越 JIT reservation 与系统 DLL 的 >2 GiB 距离。日志 SHA-256
  `b96aef2719f394e7ced1490695127ebdcbad2a04a23483a9c96b4482c3e5cc00`，fact artifact
  `9758367296`。
- 用本机 Clang 实际生成的 x64 COFF 诊断证明：volatile internal function-pointer slot 使外部
  三 symbol 变为 `IMAGE_REL_AMD64_ADDR64`，本地 call 只保留对 slot 的 REL32；这支持“64-bit
  pointer + local stub”机制，但不是 production/remote 签收。正式修订应在 COFF-x64-only
  JITLink graph pass 使用 LLVM 官方 R-only pointer/RX stub primitives，不修改共享 runtime
  source、AOT artifact、cache recipe 或 process-symbol allowlist。

## I35：Windows dependency audit 把 candidate 路径误当 import name

- 同一 run 的 ARM64 job `99491674138` 已通过 fact 7/7、Native 92/92、CLI 22/22 和 static
  release build；pinned inspector 成功执行后，regex 拒绝了整份输出。该输出首行是绝对
  `File: C:\a\Rust_CalcKernel\...`，必然命中原本用于拒绝动态 compiler dependency 的
  `CalcKernel`，并不能证明 import table 有 forbidden DLL。日志 SHA-256
  `48a78a2fa36f4db15ec6415fb314382d808597c157c7af1465e5880fa8f7405c`，fact artifact
  `9758395786`。
- pinned LLVM 22 的 `--coff-imports` 同时列出 regular `Import`、`DelayImport`、文件元数据与
  symbol/RVA；审计必须 fail-closed 提取两个 descriptor scope 的 `Name:`，只对依赖名应用
  原 regex。不得删 `CalcKernel`、过滤特定 workspace 路径、跳过空/畸形输出或退回 PATH 工具。

## I36：Windows native artifact audit 重复依赖 `dumpbin` PATH

- 精确 SHA `6dcd2ce` 的 run `33397814019` 中，Windows x64 job `99506470952` 已完成
  fact 7、Native 92、CLI 22、static release build，且 I35 修复后的 release dependency audit
  明确 passed；下一步 `scripts/audit-native-artifact.ps1:31` 的 `Get-Command dumpbin.exe`
  因 runner PATH 未初始化失败。完整日志 SHA-256
  `9b10eec294ba922bc2f9934c64b6108bf662ba113257f70a5072807aae0f503b`。
- 同一 run 的 Windows ARM64 job `99506471267` 到达相同的唯一失败点，完整日志 SHA-256
  `1ccfd53449e985393d152da27ec7744e8e5fe664f91429616d063e6a560014f3`；run 已自然结束
  8/10 success。两份日志共同证明问题与 candidate architecture 无关，但不构成跨 SHA 验收。
- 该脚本尚未读取 program/module/runtime candidates，不能把失败解释为 artifact 内容不合格。
  根因与 I33 同类但位于独立审计面：必须改用已验证 prefix 的绝对 `llvm-readobj.exe`，并保持
  imports/exports/symbols 三类语义与全部 fail-closed 门。不得通过初始化任意 SDK PATH、删除
  artifact audit、放宽 `kernel32.dll`/export/symbol allowlist 或把 Windows job 标 optional 修复。
- 计划见 Task 15。当前 run 的剩余 jobs 必须自然终止；后续只由修复后同一 SHA 的完整十项
  矩阵签收 I36/I35/I34 及此前远程项。

## I37：Linux PowerShell ANSI 颜色破坏测试诊断匹配

- 精确 SHA `bcfb4ffd6307b3a154a4b8b9a94595dcb430bd58` 的 run `33400680042` 中，quality
  job `99515949231` 唯一失败为 Windows artifact audit 的 Unix fake-inspector contract。
  product audit 正确 nonzero，stderr 含预期 `dependencies must be exactly kernel32.dll`，但
  Linux PowerShell 插入的 ANSI SGR 把相邻词分隔；现有测试仅删除 `|`/折叠空白后无法匹配。
  完整日志 SHA-256 为
  `a551bce289a2685283eae897d64f3021a3227e4b3502cae3c293943066bf91ac`。
- 这是测试输出规范化缺陷而不是 product/CI 环境缺失。修复只允许剥离标准 ANSI SGR 后继续
  精确 message assertion；不得改 audit、删除诊断检查、只验证 nonzero 或取消 quality job。
  计划见 Task 16；当前 run 的其他 jobs 必须自然结束，后续仍需新 SHA 完整十项矩阵。

## I38：symbol parser 忽略真实 `Symbols [` 容器

- run `33400680042` 的 Windows x64 job `99515949259` 与 ARM64 job `99515949274` 均通过
  release dependency audit，随后 artifact audit 报告 no symbol descriptors；日志 SHA-256
  分别为 `02a1f34079f8938dd40ac10635d60bc5e41516adf370317ed978b63953ef0680` 与
  `d7bb6782c8ca5f8aab2e41dc2e823ed3769ea39a9c214658f6bfec8d962d0202`。
- pinned LLVM 22 对真实 COFF 的结构是顶层 `Symbols [`、缩进 child `Symbol {}`、更深层直接
  `Name:` 和 auxiliary scopes；本机真实 probe 日志 SHA-256
  `fc9442257b3aa93371024252ca29a494322062f00ee85a30839464267c76f034`。I36 的通用 parser
  只接受列首 brace scope，故错误地产生空集。两个架构一致失败证明是格式模型错误。
- 修复必须增加容器感知、scope-aware、fail-closed parser；不得取消空集拒绝、扫描所有缩进
  Name、回退 raw regex 或放宽 forbidden symbols。计划见 Task 17，后续仍需新 SHA 全矩阵。

## 修订边界（全部阻断，持续有效）

- 同步修订 Native LLVM ABI 与 release 双语文档、阶段 11 task/acceptance 和仓库契约测试。
- 不跳过任何 Native/JIT/cache/run test，不把失败 job 改成 optional，不降低性能门槛。
- 本轮修订必须在同一 commit 上重新通过 quality、native integration、六 native host 与两
  performance runner；在此之前不能把各轮本地或部分 host 成功汇总为远程验收完成。

## 修订后对抗性复审

### I34/I35 实现后本地复审

- I34 使用 LLVM 22 的官方 x86-64 anonymous pointer/jump-stub primitives；原 COFF `PCRel32`
  只改指向同 graph 的 RX stub，真正 process address 由 R-only cell 的 `Pointer64` 承载。
  pass 位于 COFF lowering 之前的 PostPrune，后续官方 PreFixup 仍负责把平台 edge 降为通用
  x86-64 edge。三项外部 symbol、direct-call opcode、edge kind、reserved section 与 pointer
  edge 均 fail closed；任意 process lookup 仍关闭。未发现扩展 ABI、runtime source/cache、
  AOT artifact 或 W^X 权限的路径。
- I35 初版虽不再扫描 `File:`/`Symbol:`，但只凭缩进抓 `Name:`，不能证明 scope。复审把它
  判为真实阻断并新增第二轮行为 red：scope 外 `VCRUNTIME140.dll` 必须忽略，同时任一缺名
  descriptor 即使旁边另有有效 descriptor 也必须拒绝。最终 parser 只进入顶层 `Import {}` /
  `DelayImport {}`，按 brace depth 收集唯一直接 `Name:`，未闭合、缺名、重复、非法 DLL、
  空集合、forbidden 名与 inspector nonzero 全部 fail closed。该补强没有删除或放宽原 regex。
- 最终全量本地与冻结 schema-6 只读复验通过；当前未发现新的本地 blocker。仍不能据此关闭
  I25–I35 或阶段 11：I34 的真实远地址执行、I35 的真实 PE 输出和双 Windows release audits
  必须由同一最终 SHA 的十项远程矩阵证明。

### I36 实现后本地复审

- 实现提交 `1b842e32272325cf88304eb558c245ef363ea0d4` 改用已验证 prefix 的绝对
  `llvm-readobj.exe`，保留 imports/exports/symbols 与所有原 allowlist/hash 门。首轮旧脚本
  behavior red 与实现 green 后，复审没有停在表面 source contract：新增 scope 外
  `File: C:/free/runtime.obj` 对照，证明扫描原始 symbols 文本会把路径误当 forbidden `free`。
- 第二轮实现统一按顶层 scope/brace depth 收集唯一直接 `Name:`，空 symbol table 也拒绝；
  malformed/unclosed/duplicate/illegal name 与 inspector nonzero 均 fail closed。此修订不改变
  artifact、linker、ABI、cache、CI 状态或 forbidden 集合。完整本地门与冻结性能复验全绿，
  当前未发现新的本地 blocker；I36 和此前远程项仍等待同一 SHA 的十项矩阵签收。

### I37 实现后本地复审

- `b57734e44e855c32a6cef89138f5a28af4dee053` 只在测试侧剥离严格 SGR，仍匹配完整
  PowerShell 诊断；未知 escape 不删除，产品脚本和所有拒绝门不变。固定 red→green 与完整本地
  门通过，未发现新的 I37 本地 blocker。
- 远程自然结束后，quality/native-integration 已确认同属 I37；双 Windows 则独立暴露 I38，
  不能因为 I37 已修复而签收本轮 6/10。I38 必须按真实 COFF probe 另行闭环。

远程复审重点保持为 Windows archive/CRT identity、
Darwin 两条路径的 W^X 互斥性、audit 是否可能接受不一致 tuple、TypeScript oracle 配置
是否仍跨 job 泄漏、performance 分母是否确为摘要固定的 V0.10 C source、release
no-change cache 是否独立核对完整状态而非信任 pass change declaration、guard-free demand skip 是否会
漏掉安全消费者、Darwin object 是否没有 absolute text fixup、dyld C-ABI entry/exit 是否
正确、runtime cache 是否可能命中旧 source、Windows checkout 是否保持 provenance
字节，以及是否有测试被跳过。
