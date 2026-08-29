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
- CI 增加可在 feature branch 显式触发的 `workflow_dispatch`，六 native-host runner 都执
  行 pre-LLVM fact audit；x86-64/AArch64 performance runner 执行全部新 gate，并通过同机
  冻结 Clang oracle 校准 hosted runner 共模漂移。
- Host bootstrap 必须把 Windows CMake 明确绑定到 MSVC，不能接受 GNU `.a` 冒充 MSVC
  `.lib`。Darwin JIT 必须按运行时能力选择 `MAP_JIT` 线程级 W^X 或普通 mapping 的页级
  RW-to-RX/R-NX；两条路径都运行完整 Native suite 和 memory audit，禁止 RWX fallback。
- 可选 TypeScript oracle 的 checkout、build 与 `CALCKERNEL_TS_ROOT` 必须同属 quality job；
  Native/release jobs 保持 self-contained，不能继承一个不存在的 oracle 路径。

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
