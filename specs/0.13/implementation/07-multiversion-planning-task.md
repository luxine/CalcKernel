# 阶段 07 任务：target set、variant planner/checker 与 KIR bundle

## 目标

实现 `--cpu multiversion` 的编译期部分：closed `KirMultiversionTargetSet` schema 1、显式 feature-level
target machines、eligible-root/variant proposal、独立 profitability/feature/growth checker，以及包含一份
baseline、每 root 至多两个 enhanced implementation 和 closed dispatch plan 的 verified KIR bundle。

## 仓库落点

- 修改 `src/backend/llvm/{target.rs,profile.rs,ffi.rs}`，新增 `multiversion.rs`；扩展
  `NativeCpu::Multiversion`、显式 target triple/cpu/features、target-set canonical digest 与 cost profile。
- 修改 `src/ir/kir/{model.rs,print.rs,verify.rs,mod.rs}`，新增 `multiversion.rs` 表示 verified bundle、
  per-variant proof/profile/codegen digest、hidden symbol map 与 dispatch plan。
- 修改 `src/optimizer/{kir_pipeline.rs,audit.rs,verify.rs,cost.rs}`，新增 root/variant planner、closed
  record/checker、shared growth ledger 与 stable ranking。
- 新建 `tests/multiversion.rs`、`tests/optimizer/multiversion.rs`，扩展 Native target/profile/LLVM
  feature audit和 CLI emit-kir tests。

## TDD 顺序

1. 写 target-set RED：x86-64 Linux/Darwin/Windows baseline/v3/v4；AArch64 Linux baseline/SVE/SVE2；
   AArch64 Darwin/Windows baseline-only。feature list、OS state predicate、LLVM identity、layout/cost/
   digest、schema/order 固定，table 改变必须进 identity。
2. 写 CLI/inspection RED：Native emit-kir 接受 multiversion O3并打印完整 verified bundle；build路径
   能解析、验证并到达 internal bundle builder（production final装配在阶段09启用）。O0–O2、C/Wasm/
   default inspection、sanitizer、`build-llvm` 和 `--kind object` 明确拒绝且不输出。inspection 不按
   build host 剪 variant。
3. 写 eligibility RED：root 仅 exported/entry、reachable nonrecursive、Native-supported；target-dependent
   benefit 同时 >=10% 且 >=2 units；有 profile 时只考虑 PGO-hot，无 profile 用 ordinary static cost。
4. 写 variant RED：所有 variant 从同一 verified logical KIR pre-state 独立生成，不能从另一个 variant
   派生；hidden helper 可 clone/inline 但不 export，每 variant 有独立 proof/cost/feature/size digest。
5. 写 budget RED：每 root baseline + 0..2 enhanced；additional KIR units <= complete post-O3 baseline
   units，最终 <=2x；与 PGO/0.12 clone/transaction ledger 共享，trial/non-winner 不退款。
6. 写 ranking RED：dynamic cost、smaller size、fewer features、tier identity、root identity total order；
   v3 可优于 v4/baseline，baseline-only 给 `no-compatible-enhanced-tier`。
7. 写 checker mutation RED：forged feature/benefit/size/budget/pre-state/proof/mapping/symbol/variant order
   均 withholding bundle；unsupported transform 是 stable baseline fallback。
8. 实现 canonical tables、explicit target profile queries、planner/independent checker/KIR bundle printer；
   分别 lower/audit baseline 与 variant LLVM module，但不在本阶段实现 production dispatch/packaging。

## 实现边界

- SVE/SVE2 仍只 lowering 既有 fixed-width Vector KIR，不增加 scalable value/public ABI。
- 不猜微架构，不按 build host feature 偷换 target；`native` 仍是单一 exact local CPU policy。
- variant 之间禁用 LTO；阶段 07 的 separate module bytes 只作下一阶段输入，不先 partial-link。
- feature branch 的阶段 07/08 检查点不宣称 multiversion `build` 已可交付；只有阶段 09 完成 real
  linker/archive 与原子 output transaction 后才允许 CLI success。

## RED/GREEN 证据

记录 target-set golden digests、baseline-only 与 enhanced accepted/rejected cases、shared budget ledger、
per-module feature audit 到 `target/acceptance/v0.13/stage-07/`。
