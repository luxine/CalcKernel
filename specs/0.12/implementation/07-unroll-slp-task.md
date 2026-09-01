# 阶段 07 任务：受控展开与 SLP

## 目标

实现 scalar full/partial unroll 与 source-order identity SLP 的 proposal、independent checker、
transactional materializer。C/Wasm 可提交 scalar-only winner；Native proposal 在阶段 08 的
Loop SIMD frontier 完成前不得抢先提交。

## 仓库落点

- 新增 `src/optimizer/kir_passes/{unroll,slp}.rs` 与相应 analysis/cost modules。
- 扩展 closed `UnrollPlan`/`SlpPlan`、proof steps、audit/stats/explanations。
- 测试：`tests/optimizer/kir_o3.rs`，新增专门 unroll/SLP test modules 并接入 aggregator。

## TDD 顺序

1. 写 full-unroll RED：constant trip 0..8、body <=16、LCSSA/phi/exits、zero/remainder、growth；
   trip/body/growth 邻界外拒绝。
2. 写 partial-unroll RED：factor 2/4、exact/remainder、branch savings；10%+2 unit 边界；order-
   sensitive effect/guard/call/possible failure 不跨 stop point 复制。
3. 写 unroll checker mutation RED：iteration coverage、order、phi/LCSSA、remainder、cost/growth/
   budget 任一伪造拒绝；ID exhaustion/预算耗尽整 proposal 回滚。
4. 写 SLP discovery RED：同 block、isomorphic、independent、同 lane type/semantics、source-order
   identity packing；splat/arithmetic/compare/cast/select/contiguous memory。
5. 写 SLP barrier RED：guard/call/runtime/print/unknown write/block edge/certificate dependency、
   非连续或逆序 memory、shuffle 需求、f64 horizontal、partial call 全拒绝。
6. 写 combined transaction RED：只有 unroll+SLP 合计过 10%+2 时两者原子提交；任一 checker/
   budget/post-verifier 失败两者都不提交，audit 仍计费。
7. 写 C/Wasm scalar-only integration RED：profile vector-disabled 时仅允许 independently profitable
   scalar full/partial unroll；无隐藏 Vector KIR。
8. 写 Native staging RED：生成并验证 scalar/SLP alternatives，但 production module 在没有
   Loop SIMD frontier 比较前保持原 pre-state；阶段 08 再启用 winner commit。

## 实现边界

- SLP 无 shuffle/gather/scatter/masked memory/vector call，memory pack footprint 不扩宽。
- Factor/VF/UF、cost 与 variant key 使用固定有序枚举；non-winner 不退款。
- Unroll 不能复制 observable failure/call 到原程序可能先停止的位置。
- 不实现 Loop SIMD、versioning 或 reduction；不以 LLVM SLP 结果代签 KIR SLP。
