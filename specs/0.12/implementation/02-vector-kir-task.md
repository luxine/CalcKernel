# 阶段 02 任务：Vector/Mask KIR、结构验证与 closed plan schema

## 目标

把 KIR value 类型迁移到 `KirValueType`，加入设计冻结的 fixed-vector/mask 指令与 closed
proposal record；尚不生成或 lowering Vector KIR。所有现有 scalar producer/consumer 必须
通过显式 `Scalar(MirType)` 保持等价。

## 仓库落点

- `src/ir/kir/{model,builder,print,validate}.rs`。
- `src/optimizer/{proof,verify}.rs`，可新增 `src/optimizer/vector_plan.rs`。
- C/Wasm backend 的 exhaustive match 只接受 scalar KIR，并在遇到 vector 时返回 typed error；
  Native exhaustive match 与真正 lowering 一并留到阶段 03。
- 测试：`tests/ir/{kir,proofs,memory_ssa}.rs`、三个 backend 的结构拒绝测试。

## TDD 顺序

1. 写 scalar migration RED：instruction result/block parameter 为 `KirValueType::Scalar`，function
   params/return/call/export storage 仍是 `MirType`；既有 builder 输出和 printer 稳定。
2. 为 splat、contiguous load/store、binary、unary、compare、select、cast、insert/extract、
   modular reduction 逐族写 printer/validator RED；覆盖所有合法 lane 和 unsupported 组合。
3. 写 mask/region escape RED：lane mismatch、mask-as-int、vector function ABI、region 外 block
   edge、consumer vector-disabled、profile digest stale 必须拒绝。
4. 写 vector memory RED：lane footprint、known/required alignment、Memory SSA in/out、slice end
   精确覆盖；伪造 footprint、越界 widening、错误 region/version 必须拒绝。
5. 写 closed `VectorizationPlan`/`SlpPlan`/`UnrollPlan`/`SpecializationPlan` 数据模型与 canonical
   printer RED，包含 pre-state identity、cost/growth/budget、proof roots，不接受开放字符串 op。
6. 写 proof schema/mutation RED：错误 lane map、reduction semantics、fallback/epilogue root、
   target profile identity 拒绝。实现仅提供 checker 可消费的闭合记录，不写 proposer。

## 实现边界

- Vector lane 仅 i32/i64/u32/u64/f64；mask 独立；lane count 正 u16 且必须在 profile 中 Legal。
- 无 vector bool/pointer/slice/struct/void，无 gather/scatter/shuffle/masked memory/vector call。
- f64 无 Mod/水平 reduction/FMA/reassociate；cast 仅 i32/u32 到 f64；integer Div/Mod 必须有
  no-failure proof 与 Legal target op。
- 本阶段不允许任何正常 pipeline 产生 vector instruction；backend 的 typed rejection 不是
  scalarization fallback。

## RED/Green 证据

按 instruction family 记录至少一个正确 RED 与一个 mutation RED。大型 enum migration 必须
分小步保持 `cargo check` 可解释，不能一次性以大量无关编译错误充当 TDD。
