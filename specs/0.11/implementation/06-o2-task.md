# 阶段 06 任务：O2 跨过程与 Memory SSA 优化

## 目标

在 O1 基础上实现 effect-aware inlining、GVN、Memory-SSA load forwarding、dead-store
elimination，再执行 propagation/check-elimination 与 cleanup。

## 仓库落点

- `src/optimizer/passes/{inline.rs,gvn.rs,load_forward.rs,dse.rs}`。
- 扩展 fact/proof invalidation、contract call-instance clone 与 effect summary recompute。
- `tests/optimizer/kir_o2.rs`。

## TDD 顺序

1. 写 exact O2 pass-order red test。
2. 写 safe small inline、void call、multi-return、recursive/noinline-budget、print/may-fail
   red tests；实现 deterministic cost budget 与 CFG clone。
3. 写 unsafe inline scope red tests：按 call instance 替换 facts，只支配 clone，caller
   其他 path/recursive edge 不可见；先用 mutation 证明 scope 扩大被 verifier 拒绝。
4. 写 GVN red tests：integer/value/strict f64、dominance、memory version、may-fail barrier、
   alias third-root；实现只合并完全等价 expression key。
5. 写 load forwarding red tests：same partition/version 正例，以及 alias write、unknown
   call、join/loop、volatile-equivalent ordered effect 反例。
6. 写 DSE red tests：被覆盖 store 正例，external observation、alias read、call、return、
   print/may-fail order 反例。
7. 写二次 propagation/check elimination，证明 inline/forward 后新产生的 bounds proof 能
   删除 guard，同时旧 proof 全部重验。

## 实现判定

- inline 前后 observable call/print/failure 次数与顺序相同。
- unsafe contract facts 永不提升为 caller-entry facts。
- GVN/load forwarding/DSE 只能通过共享 alias/effect service 和 Memory SSA version 决
  策，不实现 pass-local alias heuristic。
- 成本预算由 KIR size/config 决定，不依赖墙钟。

## 明确不做

不做 loop transform、unroll、SIMD、specialization 或 target-specific profitability。
