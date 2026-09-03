# 实施阻断复诊 09：Predicated Update 的规范 CFG 形状

## 复现

阶段 14 的首个 RED 使用冻结语义源码
`if candidate < old { dst[index] = candidate }` 进入现有 KIR v3
canonicalization。实际不可变 pre-rewrite KIR 由四个循环内基本块组成：header、
conditional body、唯一 store arm、merge/latch；空 `else` 不生成独立基本块，body
的 false edge 直接进入 merge/latch。阶段任务却要求“五 block canonical loop”并在
候选元数据中强制一个不存在的 `else_block`，因此任何由 CK 前端产生的目标源码都会
以 `unsupported-vector-loop-shape` 被拒绝。

同一复诊还确认，阶段任务要求把 compare/load/store 写入 `CandidateKey`，但冻结的
Loop SIMD key 已由 `(function, loop, class, variant, VF, UF)` 唯一标识 site/variant；
改写该 key 会波及现有 unroll/SLP frontier，且与阶段 13“不改 wire/schema”的约束
不一致。scalar root 的精确身份应由候选证据和阶段 16 source-aware attestation 绑定，
而不是伪装成新的 wire 字段。

## 判定

阻断成立。这是计划对仓库真实 CFG 与冻结 key 边界的错误假设，不是实现缺陷，也不是
性能门槛失败。若不修订，目标能力在真实 CK 源码上不可达。

## 修订边界

- 规范语义保持不变：仍只接受一条路径恰好一次 same-place store、另一条路径无
  memory/effect，并物化 compare + select + 单一 unmasked store。
- recognizer 绑定 CK 当前产生的四块空-else规范形状：body false edge 直接进入
  merge/latch，store arm 也跳入同一 merge/latch；不得因此接受 general diamond。
- 候选只记录真实存在的 store arm 与 merge/latch，不制造 `else_block`。
- `CandidateKey::LoopFrontier` 保持冻结；compare/load/store、branch polarity 与
  Memory SSA 身份进入候选证据，并由独立 checker/attestation 重建。
- alias、effect、strict-f64、checked proof、VF/UF、性能阈值和 CI 门槛均不改变。

