# 阶段 11 任务：全仓切换与 0.11 候选硬化

## 目标

删除正式双优化路径，完成生成/差分/mutation/性能/文档/版本/六 host CI，使分支成为可
审查但尚未发布的 0.11.0 候选。

## 全仓切换

- 所有 C/WASM/Native/benchmark/test helper 改用 `compile_kir` 或显式 KIR API；`emit-mir`
  单独保留 semantic MIR lowering/printing。
- 删除旧 MIR optional pass pipeline 和 backend 直读 optimized MIR 路径；可保留 MIR
  validator/reachability primitives，但不能有第二套 target-neutral optimizer。
- 加 repository contract test，扫描正式 backend/CLI 不得依赖旧 `MirPass` API。

## 验证资产

- 增加 fixed-seed generated kernels：O0 对 O1–O3，C/WASM/Native，所有 supported modes；
  contract cases 只生成满足 declared domain 的输入。
- 扩充 KIR mutation corpus：dominance、scalar/memory phi、partition、stale fact、ProofId、
  ordered failure/print、fact audit。
- 在 release profile 直接执行 pass-preservation 故障注入；验证缓存必须核对完整状态，
  不可把 release 的内部 change 标记当成可信证据。相关回归加入无需 LLVM 的 quality job。
- 增加 canonical proof-loop corpus，同时结构检查 KIR/C/WASM/LLVM hot loop guard。
- 新增 `benches/baselines/v0_10_compiler.toml`，固定 commit
  `df816502876fba41676f9ebc190e4fadd18cd5a5`、source digest、compiler identity、LLVM、
  target/CPU/mode/harness/statistics、配对 Native/Clang median，以及由精确 V0.10
  compiler 生成且摘要固定的 C oracle source。
- 扩展 benchmark schema/checker：既有 Native/Clang 95% geo 与 10% individual；0.11 对
  0.10 的配对归一化比率
  `(T0.11-Native/Tcurrent-Clang)/(T0.10-Native/T0.10-Clang)` 最多 3% geo/8%
  individual 回退；两个 Clang 项必须编译同一冻结 V0.10 C oracle，不能使用候选自己
  导出的 C 抵消 KIR 回退；proof-loop checked 至少 97% unchecked geo；KIR optimize time
  中位最多 2x、individual 最多 3x 0.10 MIR optimize。
  I14/I19 修订按 `11-runtime-replay-plan.md`：上述四项运行时计时在同一进程重放，
  使用独立固定 0.10 编译器生成的产物与八通道双模式交错采样。历史配对 median
  保持不变作 provenance，不再代替实际重放分母；新增完整产物摘要和采样顺序校验。
- CI 增加可在 feature branch 显式触发的 `workflow_dispatch`，六 native-host runner 都执
  行 pre-LLVM fact audit；x86-64/AArch64 performance runner 执行全部新 gate，先准备
  固定编译器 bundle，再对双版本及冻结 Clang oracle 同进程采样，不假设跨 worker
  的 Native/Clang 比率恒定。
- Host bootstrap 必须把 Windows CMake 明确绑定到 MSVC，不能接受 GNU `.a` 冒充 MSVC
  `.lib`。Darwin JIT 必须按运行时能力选择 `MAP_JIT` 线程级 W^X 或普通 mapping 的页级
  RW-to-RX/R-NX；两条路径都运行完整 Native suite 和 memory audit，禁止 RWX fallback。
- Git checkout 必须固定文本 LF、vendor provenance 原始字节；Windows 的 `core.autocrlf`
  不能改变许可证、源文件或 manifest 的摘要。不得通过放宽 checksum 或校验内 normalize 修复。
- Bootstrap cache identity 必须覆盖 LLVM manifest、两个 bootstrap recipe 及全部 runtime
  source/header/assembly/platform link input；新 prefix 在自身 manifest/object hash 验证后立即
  保存，不能等待下游测试成功，也不能让 runtime 修订命中旧 object。Darwin AOT/ORC object
  必须统一使用 PIC 与 Small code model，禁止 internal call 产生 absolute text fixup；
  `LC_MAIN` 使用 dyld 普通 C-ABI 调用的 compiler-generated `_main`，runtime failure
  仍由 platform exit helper 终止。撤销 I09 已被否定的 raw-stack entry 假设。
- Cache boundary 用独立 verifier 解析准确字段并验证五个 runtime object 的 SHA-256、
  Windows import hash 与声明的 static libraries；release prefix 验证/保存必须先于
  oracle build，后者失败不能使已完成的 release prefix 丢失。
- 可选 TypeScript oracle 的 checkout、build 与 `CALCKERNEL_TS_ROOT` 必须同属 quality job；
  Native/release jobs 保持 self-contained，不能继承一个不存在的 oracle 路径。
- ELF artifact audit 必须分别证明 loader-visible dependency 与 producer provenance：linked
  `.comment` 只允许 non-`ALLOC` 且必须包含 pinned `Linker: LLD 22.1.8`，不能把该 marker
  当成动态依赖，也不能删除版本/provenance 检查。Darwin hardened entitlement audit 必须用
  documented XML extraction 和 canonical plist 等值验证唯一 `allow-jit` policy，不能解析随
  macOS 版本变化的人类可读输出。
- Darwin 的 CI/release job 必须显式为实际 compiler 添加 ad-hoc hardened 签名及唯一
  allow-JIT entitlement，再执行严格签名审计；不能假设 linker 在 Intel 上自动签名，
  也不能仅验证临时签名副本却打包未签名原件。

## 版本、ABI 与文档

- Cargo/package/version tests 更新为 `0.11.0`，但不创建 tag/Release。
- Native public ABI 保持 1；private LLVM bridge ABI=2；contract runtime helper 使 Runtime
  ABI=2；cache/codegen contract 使用 KIR v1 identity。
- 添加 `tests/fixtures/compatibility/v0_11/manifest.toml`，逐项映射 unsafe contracts、KIR
  inspections、sanitizer、fact audit 与保留 0.10 source 的 test evidence。
- 把实现后的稳定语义同步并入所有相关 current English/zh-CN docs：language、diagnostics、
  CLI、MIR/KIR boundary、optimizer、architecture、C/LLVM/WASM ABI、modes、performance、
  compatibility、getting started、release/checklist/roadmap/index。
- 本分支保留 `specs/0.11` 的规范、审查和实施证据供用户 review；它们只在实际发布时按
  规范另行删除，当前任务不得预先销毁用户要求的计划。

## TDD 与执行顺序

I20 的无用标量 phi 清理补充计划见 `11-ssa-phi-pruning-plan.md`，先观察行为 red，
再验证闭环、双边输入、元数据/契约根与 malformed 输入原子性；不能替代以下完整门禁。

I21 按 review 中已复现的 SDK 宏冲突执行：先为真实 bridge COFF 分支加入 Unix
macro-header syntax regression，再隔离 Windows include 宏，保留 SDK ABI 与 LLVM
枚举；必须另行通过两架构 Windows 实际 SDK/MSVC 构建，不能以模拟 header 替代。

I22 按 review 中已复诊的 Windows C API DLL 默认值执行：先复现 bootstrap 的 bin
目录漏检与缺少 `LLVM_BUILD_LLVM_C_DYLIB=OFF`，再修正 Windows 配置及 bin/lib 安装
断言。独立 cache verifier 对真实 DLL 注入保持拒绝，沿用全部 recipe 的缓存身份；
不得删除 DLL、复用不合格旧 prefix 或降低静态依赖检查来通过门禁。

I23 按 `11-interrupt-handoff-plan.md` 修复 Unix run 的 SIGINT 登记竞态；先用真实
生产模块和隔离进程复现丢失，再实现单原子 pending 交接。原 public test 仍只发一次
SIGINT 并严格要求 245/CKR0006，增加有限失败期限和自有进程清理，不以延时掩盖问题。

I24 补齐切换时遗漏的三项 legacy preservation 回归，保留阶段 02 原测试过滤器，
在 verified KIR 的 O0–O3 路径断言控制流、typed print 顺序和 checked-bounds slice
调用/返回。原命令必须实际运行 3 项测试；详细测试限定计划见 review，不恢复旧优化路径。

I25 按 `11-windows-static-link-plan.md` 修复真实 Windows 全链 CRT 不一致及 COFF
closure 缺失。先保存 CMake 未使用旧参数、LNK2038 与六个 LNK2019 的原日志，再以
实际 COFF archive red/green 校验 producer/cache 两个边界。Rust debug/test/release
统一静态 CRT；旧 Windows cache 不合格，新键完整重建，仍须同 SHA 全十项 CI。

1. 先写 repository scan、version/ABI/compat manifest red tests，再完成全仓切换与版本。
2. 先写 generated/mutation corpus 的故障注入，再实现 harness，固定 seed/schema。
3. 先用 synthetic report 证明四类 performance threshold 会分别拒绝，再跑真实测量。
4. 先写 docs parity/required-term red tests，再同步双语文档。
5. 本地总验收通过后推送 feature branch，显式触发 CI 并等待所有六 host 与 performance
   jobs；修复真实失败，绝不改低阈值。
6. 若真实 runner 暴露 toolchain/capability 假设错误，先在 review 文档保存原 job/log 与
   复诊，再用失败契约测试锁定正确边界；修复后必须重新跑同一完整 matrix。

## 明确不做

不合并 main，不创建 PR/tag/Release，不增加 0.12+ 功能，不把 CI 未运行写成“已通过”。
