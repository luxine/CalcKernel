# 阶段 07 任务：O3 循环与归纳优化

## 目标

实现自然循环/归纳分析、effect/alias-aware LICM、induction simplification 和最终
propagation/check-elimination。只做规范列出的标量循环工作。

## 仓库落点

- `src/optimizer/analysis/loops.rs`。
- `src/optimizer/kir_passes/{licm.rs,induction.rs}` 与独立检查器 `src/optimizer/verify.rs`。
- `tests/optimizer/kir_o3.rs` 与 canonical checked-loop fixtures。

## TDD 顺序

1. 写 exact O3 pass-order 与 natural-loop tree red tests，覆盖 nested loop、多个 latch、
   break/continue、irreducible fallback。
2. 写 induction facts red tests：i32/u32/i64/u64、ascending/descending、strict bound、step、
   zero trip、wrap risk、contract-limited trip count；实现 widening 后的 certificate。
3. 写 LICM red tests：pure invariant 正例；may-fail、print、alias load/store、unknown call、
   non-dominating exit、checked first-error 反例。
4. 写 induction simplification red tests，保证 modular/checked overflow 边界分别合法。
5. 写 canonical slice loop：在 O2/O3 删除 hot-loop bounds guard；无契约/错一步/alias effect
   的近邻反例必须保留。
6. 写 generated small-loop fixed-seed semantic comparison，覆盖 break/continue/nested loop 和
   checked error order。

## 实现判定

- LICM 不把 may-fail 或 observable operation 移过循环入口/迭代中的其他 ordered effect。
- loop proof checker 验证 entry 与 transfer edge；分析无法收敛或超预算时保守退出。
- 不做 loop canonicalization 的额外语义 transform、unroll、vectorization、versioning 或
  target cost model。

## 明确不做

不引入 SIMD/SLP，不产生 target CPU 特化，不改变 strict f64 reassociation。
