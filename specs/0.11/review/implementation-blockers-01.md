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
  最终判定仍必须由修复 commit 上的 x86-64/AArch64 完整 performance jobs 给出。

## 修订边界

- 同步修订 Native LLVM ABI 与 release 双语文档、阶段 11 task/acceptance 和仓库契约测试。
- 不跳过任何 Native/JIT/cache/run test，不把失败 job 改成 optional，不降低性能门槛。
- 本轮修订必须在同一 commit 上重新通过 quality、native integration、六 native host 与两
  performance runner；在此之前 I01–I05 只算本地修复，不算远程验收完成。

## 修订后对抗性复审

待修复后的全量本地与六 host 证据完成后追加。复审重点是 Windows archive/CRT identity、
Darwin 两条路径的 W^X 互斥性、audit 是否可能接受不一致 tuple、TypeScript oracle 配置
是否仍跨 job 泄漏、performance 分母是否确为摘要固定的 V0.10 C source、release
no-change cache 是否只复用准确的 pass change declaration、guard-free demand skip 是否会
漏掉安全消费者，以及是否有测试被跳过。
