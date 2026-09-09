# 阶段 02 任务：KIR 3 profile site、effect 与 mapping

## 目标

把 workload profile 拓扑接入 canonical KIR：稳定生成 function/edge/loop/length/constant site，
在拓扑冻结后插入 dedicated profile-effect operations，并建立不把 profile count 当 proof 的
mapping/verification contract。KIR print/schema 从 v2 升到 v3。

## 仓库落点

- 修改 `src/ir/kir/{model.rs,build.rs,print.rs,verify.rs,mod.rs}`，新增
  `src/ir/kir/profile_sites.rs` 与 `src/ir/kir/profile_mapping.rs`。
- 修改 `src/optimizer/{kir_pipeline.rs,verify.rs,proof.rs}`，让 generate/use sidecar 与 verified
  program state 分离；普通 off 路径不物化计数操作。
- 新建 `tests/pgo_kir.rs`，扩展 `tests/ir/**`、`tests/optimizer/{preservation,transaction}.rs` 及
  KIR golden/schema contract tests。

## TDD 顺序

1. 写 KIR 3 schema RED：site ID、full descriptor、site table digest、profile operation/effect domain、
   immutable annotation/mapping record 均有 canonical print/order/identity。
2. 写 topology RED：critical edge 先确定性 split，再按 canonical function/location/kind/descriptor
   产生 site；源码格式/comment/map order 不改变 table，语义/CFG/consumer/safety 改变必须改变 identity。
3. 写 event RED：entry、minimal selected edge、loop trip、selected slice length、bounded constant 的
   operation 恰好一次；early return、break/continue、checked failure、递归/loop exit 映射明确。
4. 写 effect RED：profile operation 不 alias CK memory、不制造 Memory SSA barrier，但不能被 DCE、
   clone、duplicate 或越过 counted event；generate 的固定 pipeline 只接受 one-to-one transfer record。
5. 写 mapping mutation RED：forged ID/descriptor/digest、missing/extra/duplicate/reordered op、旧 CFG
   transfer、collision、profile annotation 进入 proof arena 均 withholding artifact。
6. 写 off/use RED：ordinary off KIR 无 instrumentation；use 重建完全相同 site table 但不含 counter
   writes；C/Wasm/default inspection 不获得隐藏 profile 行为。
7. 实现最小 site builder、canonical table、profile KIR op、独立 verifier 与 KIR 3 printer；迁移所有
   KIR schema/cache contract 引用并保持既有 scalar/vector tests GREEN。

## 实现边界

- counter storage、serialization 与 directory I/O 属于阶段 03。
- profile mapping 只描述 event count transfer，不能写入 range/alias/alignment/effect/bounds proof。
- `KirTargetProfile` 仍表示 target cost；新 workload site/annotation 使用 `CkProfile*` 命名。

## RED/GREEN 证据

记录一个旧 KIR v2 golden/schema RED、一个 forged mapping RED 和最终 KIR 3 digest/test count 到
`target/acceptance/v0.13/stage-02/`。
