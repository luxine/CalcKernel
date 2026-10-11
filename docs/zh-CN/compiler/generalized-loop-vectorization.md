# 循环、内存与向量化

[English](../../compiler/generalized-loop-vectorization.md)

Loop SIMD 是 Verified KIR 上按属性判断的变换。源函数、变量和 fixture 名称
不属于合法性输入。候选发现、合法性、收益估计、物化和独立验证职责分离。

## Loop model

`CanonicalLoopDescriptor` 描述循环 identity、header、preheader、latch、exit、
body blocks、嵌套关系、induction 和 trip relation。Header 的 value parameters
与 Memory SSA parameters 通过 entry/backedge operands 描述循环携带状态。
可分析的循环不自动具有变换资格：recurrence、有序 effect、未知内存关系或
target 不支持的 operation 都可能要求保留 scalar。

不可约 CFG、不支持的 exit、不完整的状态重建与 analysis budget 耗尽均保留
scalar execution。分析记录只对重建时的 KIR snapshot 有效。

## Memory access model

共享 affine access analysis 描述 source/root memory region、base、读写类别、
元素类型与字节数、alignment、slice range 和 index form。Affine form 由稳定
排序的 SSA coefficient terms 和 constant 构成。相对内层循环，外层 induction
或 invariant expression 可以作为 leaf。Broadcast、unit stride、constant stride
是可分析属性；识别属性不意味着存在对应的 vector lowering。

Affine record 能表示的 coefficient term 多于当前 generic WASM f64 materializer 接受的范围。
该路径仍要求每个 access 都能精确投影为 loop induction 加一个 invariant offset，或一个 invariant
broadcast；同时还必须满足支持的 operation 和 loop-carried-state 形态。因此，完整 affine analysis
不等于任意 affine lowering，也不是 polyhedral optimizer。

数学 affine equality 与机器 index arithmetic 必须区分。Modular index 可能
回绕，summary 本身不能证明 disjointness、distance 或 injectivity。Checker 必须
验证所需的 nonwrapping span，或保持精确的机器 index expression。消费者使用
单 induction、单 invariant 的兼容 projection 前必须验证它没有丢失信息。

## Dependence 与 effects

共享 dependence relation 使用 normalized access records 与已验证 alias 信息，
区分 RAW、WAR、WAW、same iteration、loop carried 和 unknown。Read/read pair
不负责重排可观察 write。相同地址的 load/store 只有在跨迭代访问 injective
时才可以保持 lane local。单独一个 invariant store 不因没有第二条内存指令
就被认为 independent。

未知 alias、range 或 dependence 必须保守处理。Profile 可影响候选排序、trip
估计和 cost，不能证明 memory safety、alignment、effect freedom 或 dependence。

## Candidate、plan 与 cost

`VectorizationCandidate` 组合 canonical loop、operation mapping、内存属性、VF、
UF、trip threshold 和 predicates。Target capability 决定支持的 lane/operation。
Enumeration 有界，排序和 tie breaking 稳定；cost/growth 只在合法候选中选择。

`VectorizationPlan` 记录不可变的 KIR/profile/evidence identity、scalar-to-vector
lane mapping、memory/broadcast groups、runtime predicates、epilogue、cost、growth
和 proof roots。已有 modular integer reduction 与纯 compare/select 使用同一
plan family。Strict floating reduction 继续不可用，因为重关联会改变 rounding。

Trip evidence 按固定优先级分类：exact canonical trip、未耗尽的完整 scalar analysis 得到的 proven-bounded
trip、profile-estimated trip、unknown。Profile 只影响 cost，不建立 bound 或 legality fact。Exact
candidate 按实际 trip 数、完整 vector group 和 scalar remainder 计价，并受 target 最小 vector-group
floor 限制。它仍须满足同一闭合的 20% cost 门槛。因此，低于 runtime candidate 保守 threshold 的某些
exact trip 仍可通过；其他 exact trip 则保留 scalar。

对于非 exact candidate，只有在第一个合格 vector-group 数的每种 tail 都满足 cost 要求后，才接受
runtime threshold。Proven-bounded 或 profile-estimated trip 如果区间比最大 candidate width 短，就覆盖
区间中的每个 iteration；否则使用首尾 residue window，每端最多 64 个 iteration，总计最多 128 个。
Unknown trip 令 `H` 为所有合法 candidate 的最小 threshold，`W` 为最大 candidate width（最多 64）；共享
场景是 `max(H - 1, 0)` 到 `H + W - 1`，最多 65 个。Explanation 以 `H` 为代表值；若某 candidate
自己的 threshold 高于当前场景，则按 scalar fallback 计价，vector group 和 scalar tail 都为零。同一
scope 内所有 Loop SIMD 和 loop-SLP alternative 共用有限场景集。聚合 cost 是有界排序启发式，不代表
概率估计，也不是最坏 normalized ratio。

每个场景分别计入 vector group、实际 scalar tail、predicate，以及低于候选 threshold 时走的标量路径。
对于 persistent modular-add reduction，preheader `Compare`、`Branch` 和 `Splat` 两条路径都会计入；
只有 vector path 会计入 `Fold`、scalar seed `Add` 与 finalizer `Branch`。在当前 WASM profile 下，两部分
分别为 3 与 10 个 cost unit。一般而言，scalar fallback 只计入实际 preheader 执行的工作。Cost record
使用 checked arithmetic 和闭合 `u32` 表示；trip 或字段超出范围时拒绝候选，不会饱和成看似有利的 cost。

Structural growth ceiling 仍是硬验证限制。排序另外惩罚新增 KIR unit 和确定性的 mapped-operation
展开代理，其中包含 store 等 memory group。Loop SIMD proxy 统计 mapped vector operation 与
memory/broadcast group；SLP proxy 统计 vector、setup 与 extract instruction。它是展开计数，不估算峰值
register pressure。Penalty 为 `(ceil(新增 KIR unit / 4) + ceil(展开项 / 4)) × target Branch cost`；
若 profile 不提供 Branch cost，则使用闭合的 unit fallback。该 penalty 只用于排序，并按每个有限场景
计入。Loop SIMD 与 SLP 使用相同的场景、scalar denominator、Branch unit 和 comparator。Ratio 使用
`u128` 交叉乘法比较。最终稳定顺序为现有 x86 widening-cast frontend penalty、聚合 score ratio、仅 runtime ratio、较少的
新增 KIR unit、较小的展开代理、较小 UF、较小 VF，最后按 candidate key。`insufficient-benefit` 表示
单个 vector chunk 未达到 cost floor；只有当其解释落败时，nonwinner 才报告 `code-growth` 或 `tail-cost`，
否则报告 `higher-cost-loop-alternative`。Candidate pricing failure 使用稳定的 `tail-cost` 或
`target-cost` reason。
场景聚合总量以 `u64` 静态 cost unit 饱和累加；极端输入下可能封顶于 `u64::MAX`。`u128` 只保护已存比例
之间的交叉乘法比较，不能恢复已封顶的聚合值。因此该 score 是确定性的封顶静态 cost 排序信号，不是
未封顶总 cost 的精确百分比，也不是实测 runtime 百分比。

受支持的 WASM affine lowering 将 contiguous load/store、invariant readonly
broadcast 与支持的 strict lane operations 组合。合法性依赖完整地址、每个 write
的 independence 和 operation closure。只有 invariant value 稳定且与所有 write
独立，才可跨 UF chunks 共享 load。每条浮点 lane 保持 source operation order，
不引入 FMA 或 fast math。

## Runtime predicates 与 epilogue

Runtime versioning 在进入 fast path 前检查完整条件。Address/range arithmetic
必须 total 并检查 overflow；predicate 失败进入原始 scalar path。Trip threshold、
slice span、alias、alignment、divisibility predicates 保持闭合且有界的 schema。
Alias interval disjointness 不能证明 slice 或 physical memory coverage。受支持的
unknown-alias f64 loop 在同一 predicate budget 内组合别名与完整 span guards；
更大的 guard bundle 保留 scalar execution。

Vector body 只覆盖完整 chunks。剩余迭代由有序 scalar epilogue 处理，包括很小
和不可整除的 trip count。独立 checker 验证 partition、exit state 与 fallback identity。

## Transaction 与独立 checker

Proposer 构造 trial，不修改 accepted KIR。Checker 从原始 KIR 独立重建 loop、
operations、addresses、alias roots 与 guard premises，不信任候选给出的 safe、
noalias、cost 或 dependence assertion。双方分别重建输入后，可以共享纯
dependence arithmetic。

Checker 验证物化 operation、lane mapping、内存覆盖、predicates、target legality、
scalar remainder、未修改函数、cost 与 growth。Trial 必须保持原 contract facts、
eliminated guards、evidence generation 与既有 proof records；新增 proofs 必须
精确对应 plan 的完整 proof roots。检查通过后才原子提交 KIR 和
evidence；拒绝或耗尽 budget 保留原始 scalar behavior。

## Exposure 与 backend boundary

Invariant branch exposure、纯 decision tree 识别与 boundary peeling 可暴露循环
结构。它们的 CFG/speculation proof 与 SIMD operation/memory proof 职责不同。
Boundary producer 可以具有 specialized shape；建议的 footprint 不是安全证明。

Affine checker 独立重建 modular source index，并证明每次访问都落在 guarded
span 内。证明依赖 affine coefficients、trip bound 与 total range predicate，
不要求固定的 load 数量或排列。Count multiplication 只有在 guarded domain 内
精确、不回绕时，才能解除 nonwrapping 前提。

纯 tree exposure 先证明 leaf 互斥及 continuation，再由共享 memory/dependence
检查把它们视为一个 logical write。Scalar KIR 尚无通用 select operation，因此
tree 的 control topology 与 speculative lane expression 仍需要单独 checker。
Invariant branch exposure 从 operation 与 data flow 识别角色，再向 vectorizer
提供普通循环。

Optimizer 决定变换。WASM/LLVM backend 降低已验证的 vector operations，并检查
instruction、lane、feature 与 encoding legality，不根据 workload identity 推断
高级 loop safety。C 与 WASM `baseline` 保持 scalar vector-lowering policy。

Generic stencil legality 与 span checker 并不会让 footprint discovery 变成通用机制：当前 stencil
source exposure 仍是在 boundary peeling 后识别有界的 3x3 interior。Normalization 和 decision tree
也由各自有界的 source/CFG exposure 启动；tree checker 不是通用 scalar-select 或 if-conversion
facility。

## Identity、budgets 与限制

已发布的 0.15.2 compiler 使用 vector cost/proof/budget identity 1/1/1；
0.16.0 development candidate 使用 3/2/3，
其中 proof identity 相较正式发布版本也发生变化。Cost identity 描述 candidate admission/ranking，proof
identity 描述 legality evidence，budget identity 描述有界 analysis work。Budget 包含 descriptor-root
reconstruction；它使用 indexed worklist，并将建索引与每条已访问 edge 的工作计入
`16 * kir_function_units + 64`，最多按 8192 个 function unit 计。Native cache key 包含 cost、proof 和
完整的确定性 budget identity，因此变更规则后不能复用旧优化 evidence。KIR v3、公共 CLI、源语法和
Native/WASM ABI 保持原有契约。

Affine analysis 按 KIR size 限制 work，限制 coefficient term 数量，dependence
enumeration 耗尽预算就停止。可选的 dead-scalar 识别限制为 128 条 body instructions、
256 个 values 与 512 work steps；耗尽预算时保留原计算。VF/UF 候选空间有界，
不执行 offline auto-tuning。

Lowering domain 仍小于 affine analysis domain。任意 gather/scatter、不支持的
stride/lane/cast、floating reduction、checked SIMD、复杂 recurrence 与 effectful
branch 保留 scalar execution。Boundary envelope discovery 与 control exposure
保留较窄、独立检查的契约。通用 affine containment 本身不负责发现任意多维
footprint，也不重构任意 control。能力扩展必须先保证
correctness、fallback 与有界 work，再改变这些契约。
