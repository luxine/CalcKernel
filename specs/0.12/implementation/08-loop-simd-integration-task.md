# 阶段 08 任务：Loop SIMD、versioning、frontier 与端到端语义

## 目标

实现真正的 KIR Loop SIMD proposer/checker/materializer、optional scalar fallback/version
predicate、scalar epilogue、target-bounded VF/UF 与 modular integer reduction；和阶段 07 的
alternatives 在同一 immutable pre-state 上选唯一 winner，并完成三后端/CLI/差分闭环。

## 仓库落点

- 新增 `src/optimizer/kir_passes/vectorize.rs`、vector cost/plan/legality modules。
- 扩展 pipeline、proof checker、stats/explanations、KIR builder/materializer。
- 更新 Native lowering/audit；C/Wasm 继续 scalar-only。
- 测试：optimizer、IR、CLI、C/Wasm/Native differential，新增 vector fixtures/examples 仅用于测试。

## TDD 顺序

1. 写 simple Loop SIMD RED：unit-stride map/zip、integer transform、strict element-wise f64、cast/
   compare/select、contiguous load/store/splat；pre-LLVM KIR 必须含期望 vector op。
2. 写 lane/epilogue RED：zero、short、target-specific `2*VF*UF`/`4*VF*UF` admission boundary、
   exact、remainder、maximum-safe trip；x86-64 少于四个、AArch64 少于两个完整 vector chunk 时
   保持 scalar，无 over-read/write，tail 原序覆盖每 iteration 恰一次。
3. 写 dependence/version RED：static noalias/alignment 正例；unknown read/write 需要 <=4 个完整
   conjunct；overlap/misalignment/short/address overflow 走原始 scalar blocks。
4. 写 checked RED：只有已有事实证明所有 lane arithmetic/bounds 不失败时 fast path 合法；
   possible first-error、checked reduction 保持原 scalar；fallback guard 全保留。
5. 写 control/reduction RED：单个 pure reconverging diamond if-convert；unchecked modular add/mul
   reduction 在 profile Legal 时向量化；strict f64/其它 recurrence/scan 保持 scalar。
6. 写 independent checker mutation RED：trip partition、affine map、dependence、predicate
   completeness、lane map、footprint、fallback identity、epilogue、reduction、cost/growth/budget。
7. 写 frontier RED：Loop SIMD 20%；unroll/SLP 10%+2；同 pre-state 先各自验算，再按 total cost/
   shape/VF/UF/KIR key 选一个；所有 non-winner 计费，无嵌套 version 或重复 unroll。
8. 写 residual SLP RED：只在 committed loop region 外按稳定 block/root/lane key 运行。
9. 写 CLI/explanation RED：candidate identity、selected/rejected、VF/UF/predicate/cost/growth/
   proof/reason 字节稳定；`emit-kir` Native 显示 exact profile vector KIR。
10. 写 fixed-seed differential：O0 与 O3，四 safety mode，zero/short/exact/remainder/overlap/
    disjoint/aligned/misaligned/checked failure；C/Wasm scalar outputs 与 Native 一致。
11. 用 x86-64/AArch64 pinned object disassembly tests 识别真实 SIMD mnemonic，防止 LLVM scalar
    fallback 或 LLVM 自发向量化伪造 KIR feature 成功。

## 实现判定

- Contract sanitizer 禁用 specialization/versioning/vector/unroll/SLP，保留 scalar instrumentation。
- Native baseline/native 使用同 target profile；baseline 不得用 optional ISA。
- C/Wasm 不收到 Vector KIR，但得到合法 specialization/canonical loop/scalar unroll。
- 每个 named pass 都有 changed/verified record，最终 structural/evidence verifier 必过。
- 不加入 fast math、source SIMD、shuffle/gather/scatter/masked recovery/scalable vector。
