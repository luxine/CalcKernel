# 阶段 06 任务：O3 PGO optimizer transaction 集成

## 目标

把 profile analysis 作为收益侧输入接入现有 O3 verified optimizer：bounded inlining、direct-call/
value/length specialization、unroll/SLP/Loop SIMD factor 和 layout/LLVM checked metadata。所有 guarded
fast path 保留 generic fallback，复用 0.12 proof/checker/transaction/budget，不改变安全语义。

## 仓库落点

- 修改 `src/optimizer/{kir_pipeline.rs,audit.rs,transaction.rs,verify.rs,cost.rs}` 和
  `analysis/{specialize.rs,unroll.rs,vectorize.rs}`、`kir_passes/{specialize.rs,unroll.rs,vectorize.rs}`。
- 新增 profile candidate closed records/checkers，扩展 `src/optimizer/explain.rs` 或现有 explanation
  路径；所有 CFG transform 提供 profile mapping transfer。
- 修改 `src/backend/llvm/{kir_lower.rs,passes.rs,fact_audit.rs}`，O3 only 附加 checked frequency
  metadata/attributes 并验证 exact KIR-to-LLVM map。
- 新建 `tests/optimizer/pgo.rs`，扩展 specialization/unroll/SLP/vector/transaction/differential/Native
  LLVM metadata tests。

## TDD 顺序

1. 写 pipeline RED：O3 use 的固定顺序为 identity/site validate -> immutable analysis -> O1 ->
   profile-weighted specialize -> O2 -> loop rebuild -> one-pre-state frontier -> final verifier；O2 不进入。
2. 写 inlining/specialization RED：只对 direct call/closed constant/length class，>=128 observation、
   dominant/PGO-hot、完整 guard/fallback、既有 static/growth gate 与共享 clone budget 全满足才接受。
3. 写 loop RED：trip/length bucket 只用 checker 证明的全区间 lower bound 选择 unroll/vector/interleave；
   unknown/saturated/overlap/effect/checked-first-error/strict-f64/footprint 不得被 profile 越权。
4. 写 frontier/transaction RED：所有 alternative 从同一 immutable scalar pre-state，independent checker
   重算 cost/proof/mapping/budget，stable total order 选 lowest dynamic cost，拒绝/非获胜不退款不泄漏。
5. 写 metadata RED：只有 exact KIR-to-LLVM map 后 O3 可附 branch weight/entry/hot-cold/internal summary；
   forged/stale/missing map withholding module，metadata 不能改变 alias/bounds/failure/float authority。
6. 写 differential/mutation RED：O0/off-O3/generate/use-O3 over training、held-out、adversarial inputs；
   profile alone 不能删 check、扩大 footprint、move effect、改变 first error/print 或启用 fast math。
7. 写 determinism/explanation RED：candidate order、counter IDs、guard/cost/proof/rejection/budget ledger
   不受 map/shard/build path 影响；相同输入/profile/toolchain 产生 byte-identical KIR/LLVM/object。
8. 实现 minimal PGO proposal/checker/materializer，复用现有 analyses/proofs；每种 transform 单独 GREEN
   后再接 pipeline，最后重构共享 cost/mapping/transaction 逻辑，并让 `ckc pgo build` 的 final artifact
   真正经过该 O3 profile-use pipeline。

## 实现边界

- profile observation 永远不是 safety fact；zero count 不证明 unreachable，不删除 fallback。
- PGO specialization 与 0.12 specialization 共用预算，不重置 ledger；sanitizer 完全拒绝 use。
- target variant planning 属于阶段 07；本阶段只生成单 target verified artifact。

## RED/GREEN 证据

每个 transform 保存一个 accepted RED/GREEN、一个 safety mutation rejection、一个 budget rejection，
并记录 pipeline audit digest 与 differential seed 到 `target/acceptance/v0.13/stage-06/`。
