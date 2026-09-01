# 阶段 04 任务：canonical loop、LCSSA、affine dependence 与 version predicate

## 目标

在现有 natural-loop/induction/Memory SSA 上建立可供独立 checker 使用的 canonical loop
descriptor、loop-simplify/LCSSA、unit-stride access/dependence 分类与 total runtime predicate。
本阶段只分析和规范化 scalar KIR，不生成 vector/unroll clone。

## 仓库落点

- 扩展 `src/optimizer/analysis/loops.rs`，新增 `dependence.rs`、`loop_access.rs`、必要预算类型。
- 新增 `src/optimizer/kir_passes/loop_simplify.rs` 与 total predicate KIR/model/validator 支持。
- 更新 `src/optimizer/{kir_pipeline,verify,proof}.rs`。
- 测试：`tests/optimizer/kir_o3.rs`、`alias_effects.rs`、`tests/ir/proofs.rs`。

## TDD 顺序

1. 写 loop-simplify RED：preheader、single latch/backedge、dedicated exits、LCSSA、nested parent/
   depth、break/continue；irreducible 或不可安全归一化时稳定 scalar fallback。
2. 写 descriptor invalidation RED：CFG/inlining/specialization/unroll mutation 后旧 LoopId、
   dominance、Memory SSA、contract mapping 都不能复用；重建结果确定。
3. 写 trip/induction RED：zero/exact/remainder trip、四整数宽度、wrap risk、strict/non-strict、
   multiple exits；false certificate mutation 拒绝。
4. 写 affine access RED：`base + sizeof(T)*(i+b)`、positive unit stride、same-base distance、slice
   interval、alignment；negative/non-unit stride只用于证明不相交，不成为 vector group。
5. 写 dependence RED：read/read、independent、supported modular reduction、dependent、unknown；
   unknown write pair 只有完整 footprint non-overlap predicate 才可转为 runtime-guarded。
6. 写 total predicate RED：trip/divisibility/alignment/address interval；target-width add/mul overflow
   返回 false，zero footprint 不形成 end address，predicate 不解引用内存。
7. 写 effect/first-error RED：call/runtime/print/unknown memory/guard/ordered failure 阻断跨迭代
   重排，分析预算耗尽保留原 scalar KIR 与稳定 explanation。

## 实现边界

- 只接受 innermost countable loop 的向量 legality descriptor，但 loop-simplify 可规范化其它
  reducible loop。
- Runtime predicate 最多四个 conjunct、无 disjunction；本阶段只表示/验证，不 clone fast path。
- 不从变量名或 raw pointer 来源猜 noalias；只用 existing facts/regions/Memory SSA/contracts。
- 不改变 O0–O2；O3 新 pass 必须 `changed/verified`，normal fallback 不是 error。
