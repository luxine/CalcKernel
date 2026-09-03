# 实施期设计复诊 09：Predicated Update 的真实空分支 CFG

## 阻断复诊

冻结的 predicated-update 源码经现有 CK canonicalization 后不会保留空 `else`
基本块。真实 KIR 是 header、conditional body、store arm、merge/latch 四块形状；
false edge 从 body 直接到 merge/latch。原阶段 14 的“五块”要求和必填
`else_block` 让规范目标源码本身不可发现。

此外，原任务要求把 scalar 指令身份加入既有 `CandidateKey`。该 key 已用 function、
loop、class、variant、VF、UF 唯一标识 Loop SIMD alternative；阶段 13 又冻结所有
wire/schema。精确 scalar root 应由候选证据与 source-aware attestation 验证。

## 决议

- recognizer 只接受 CK 产生的四块 empty-else 形状：conditional body 的一条边进入
  唯一 store arm、另一条边直接进入 merge/latch，store arm 也无条件进入同一
  merge/latch；store arm 除目标 store 外无其他 instruction/effect。
- `VectorPredicatedUpdate` 记录 store block、merge block、condition/load/store、
  branch polarity 和 store Memory SSA input/output，不记录虚构的 else block。
- merge 的目标 region 必须精确选择 false 路径的旧版本与 store 路径的新版本；其余
  region 两条路径必须相等。value 参数不得形成额外 varying phi。
- `CandidateKey::LoopFrontier` 保持不变；候选证据和阶段 16 的独立 attestation 精确
  绑定 compare/load/store/polarity/pre/post digest，防止错误 scalar root 被接受。

本修订没有扩大到 general if-conversion，没有降低 alias/effect/dependence、strict-f64、
checked proof、性能、证据或 CI 门槛，也不改变语言、KIR、decision 或 native ABI。

