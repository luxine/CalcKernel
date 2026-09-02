# 阶段 03 任务：候选空间、beam search 与 exact plan replay

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

从同一 verified pre-tune KIR 枚举稳定 decision sites/tuning units/finite variants，完整记录零基 expansion
trace，按冻结 whole-plan rank 和 diversity rule 得到 frontier/compile selection，并能独立重放任一 exact plan。

## 仓库落点与接口

- 新建 `src/optimizer/tune.rs`，修改 `src/optimizer/{mod.rs,kir_pipeline.rs}` 以及必要的既有
  specialization/inlining/unroll/vector/SLP transaction 接口；提供 `enumerate_tuning_space`、
  `apply_tuning_plan`、`check_tuning_plan`，输入均为 immutable `KirVerifiedProgramState`。
- 新建 `src/tune/{frontier.rs,plan.rs,search.rs}`，定义 `TuneSite/TuneUnit/TuneVariant/TunePlan`、
  `ExpansionRecord`、`SearchFrontier` 和 `run_deterministic_search(space, preset)`。
- 若 layout 需要 KIR metadata，最小扩展 `src/ir/kir/{model,print,validate}.rs`，使 layout plan 进入
  canonical KIR digest，并由 Native backend 在固定 LLVM O3 后消费；普通 empty plan byte-identical。
- 扩展 `tests/optimizer.rs` 与 `tests/optimizer/tuning.rs`，新增 `tests/tune/{frontier.rs,search.rs}`。

## TDD 顺序

1. 写 site/unit ID RED：同 source/KIR 不受 hash-map/discovery order 影响；root anchor 唯一，overlap/
   cloned helper/code-size interaction 聚类，最多 4 variants/unit、64 units、4096 sites、256 variants。
2. 写 plan payload RED：七类 payload bounds/order/digest、phase order、pre/post digest 和 choice count 完整；
   forged fact/guard/effect/feature/growth 或跨 pre-state 选择被独立 checker 拒绝。
3. 写 expansion RED：nested loop 从 ordinal 0 连续记录 legal/illegal/duplicate/growth-rejected，命中 limit
   正好停止且不退款；whole-plan dynamic/static/printed-KIR metrics 每次从 fresh pre-state 重算。
4. 写 beam/diversity RED：baseline free carry、七类固定优先、完整 rank tiebreak、compile attempt 消耗、
   quick/standard/thorough bounds 精确；同输入多次产生 byte-identical frontier 与 selection。
5. 写 replay RED：按 specialization→inlining→short-slice→Loop SIMD→unroll→SLP→layout 顺序重放，
   mandatory analyses/checkers 保持原位，选中任意非布局 rewrite 后仍执行固定 DCE/cleanup 后缀，
   early-only plan 不得重新进入 ordinary specialization/inlining；错误
   site/pre/post/plan digest fail-closed，内部 checker/compiler failure 不得降级成可跳过 illegal。
   为 specialization/unroll/Loop SIMD/SLP 等调优路径分别绕过普通静态收益判定，但保留 legality/proof/
   target/transaction/growth，并用普通 proposer 拒绝而 tuning space 接纳的 RED 锁定隔离。
6. 写 ordinary isolation RED：empty plan 的 optimized KIR 与 v0.13 ordinary O3 完全一致，O0/O1/O2、C、
   WASM、multiversion 与非 tune build 不受新 frontier code 影响；layout-only 在移除布局元数据后也
   必须与 empty-plan 普通 O3 KIR 完全一致，并确定性投影 O3 后存活/新建基本块。
7. 运行 `cargo test --test optimizer tuning_ -- --nocapture`、`cargo test --test tune search_ -- --nocapture`
   和 `cargo test --locked`，记录 RED/GREEN 与 canonical candidate-space digest。

## 实现边界

- tuning 必须能绕过 ordinary static profitability threshold，但不能绕过 legality/checker/transaction/growth；
  ordinary O3 proposer/checker 仍使用原阈值。
- 不搜索 LLVM flag/pass pipeline、CPU tier、ABI、guard、fast math 或 source-level syntax。
- 到达 expansion/compile limit 是完整有界结果；wall budget 中断完整 expansion/compile selection 不产生 decision。
