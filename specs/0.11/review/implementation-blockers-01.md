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

## 修订边界（全部阻断，持续有效）

- 同步修订 Native LLVM ABI 与 release 双语文档、阶段 11 task/acceptance 和仓库契约测试。
- 不跳过任何 Native/JIT/cache/run test，不把失败 job 改成 optional，不降低性能门槛。
- 本轮修订必须在同一 commit 上重新通过 quality、native integration、六 native host 与两
  performance runner；在此之前不能把各轮本地或部分 host 成功汇总为远程验收完成。

## 修订后对抗性复审

待修复后的全量本地与六 host 证据完成后追加。复审重点是 Windows archive/CRT identity、
Darwin 两条路径的 W^X 互斥性、audit 是否可能接受不一致 tuple、TypeScript oracle 配置
是否仍跨 job 泄漏、performance 分母是否确为摘要固定的 V0.10 C source、release
no-change cache 是否独立核对完整状态而非信任 pass change declaration、guard-free demand skip 是否会
漏掉安全消费者、Darwin object 是否没有 absolute text fixup、dyld C-ABI entry/exit 是否
正确、runtime cache 是否可能命中旧 source、Windows checkout 是否保持 provenance
字节，以及是否有测试被跳过。
