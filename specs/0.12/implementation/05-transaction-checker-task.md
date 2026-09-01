# 阶段 05 任务：transaction、audit ledger 与独立候选 checker

## 目标

实现 `KirVerifiedProgramState`/`KirOptimizationAuditState` 分层、稳定 candidate key、共享预算、
closed plan independent checker 和 O3 frontier 骨架。用 test-only candidate 完整证明 commit/
rollback/audit 行为，尚不接入真实 specialization/unroll/vector proposer。

## 仓库落点

- 重构 `src/optimizer/kir_pipeline.rs`，新增 `transaction.rs`、`audit.rs`、`vector_check.rs`。
- 扩展 `src/optimizer/{proof,verify}.rs` 与 `analysis/budget.rs`。
- `KirPassManagerResult` 保持公开结果能力，内部不再把可回滚 program state 与 audit 混为一体。
- 测试：`tests/optimizer/{preservation,kir_o3}.rs`、`tests/ir/proofs.rs`。

## TDD 顺序

1. 写 state split RED：trial mutation/rejection 后 module/facts/proofs/guards/cache/generation/IR ID
   完全等于 pre-state；audit attempt/budget/reason 增加。
2. 写 acceptance RED：checker 通过且 post-verifier 通过时只整体交换 verified state；accepted
   audit 引用 pre-state stable candidate key，不引用 trial-only ID。
3. 写 compiler-error RED：forged proof、checker/proposer cost mismatch、post-commit invalid KIR
   withholding artifact，不 fallback 到 scalar artifact。
4. 写 ledger RED：固定 proposer `64*n+128`、checker `96*n+256`，saturating u32；rejection、reuse、
   non-winner 不退款，clone 同时扣 caller/original callee；exhaustion 原子拒绝。
5. 写 ordering RED：specialization、loop frontier、residual SLP 的 stage/key 全序；逆序 storage/
   hash insertion 不改变 attempts、winner、stats 或 explanation bytes；重复 key 拒绝。
6. 写 independent checker mutation RED：cost、growth、profile digest、proof roots、lane map、fallback、
   budget consumption 任一伪造均拒绝；checker module不得 import/call proposer/dependence/cost model。
7. 接入 O3 pipeline 空 frontier/pass records，O0–O2 exact sequence 保持；无 candidate 时 artifact
   与阶段 04 等价。

## 实现判定

- Audit ledger 是单调外层状态；所有 normal rejection 仍生成稳定 reason 和 counter。
- Checker 从 pre-state 与 closed record 独立重算，不信任 proposer 的布尔 legality/total cost。
- 每个变换 transaction 在 commit 后立即 structural/evidence verify；verification cache 不依据
  未受信 preservation declaration。
- Test-only proposer 在阶段完成前不能进入 production CLI 路径。
