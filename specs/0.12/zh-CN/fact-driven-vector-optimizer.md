# CK 0.12 事实驱动向量优化器规范

[English](../fact-driven-vector-optimizer.md)

## 状态与效力

本文是 CK 0.12.0 的预发布设计契约。它定义事实驱动优化器的下一阶段，但不声称当前
0.11.0 编译器已经实现这些行为。在 0.12.0 完成实现、验收和发布前，已发布的 0.11
语言、可观察语义、CLI 行为和 public Native C ABI 仍然有效。

本文有意保持为单一 release contract。实施阶段、日期化审查、就绪记录和验收证据不
属于本文。

## 目标

CK 0.12 将 0.11 建立的已验证标量知识转化为自动数据并行代码。由编译器而不是程序员
发现 fixed-width SIMD、受控展开、循环版本化和事实驱动函数专用化。对于符合条件的
Kernel，生成的 Native 机器码应接近经过审计的手写 C/Rust+SIMD，同时精确保留 CK 的
安全、浮点、效果和 ABI 语义。

本版本包含五项相互关联的交付：

1. canonical loop form 与循环访问/依赖合法性；
2. 确定性的目标能力和收益模型；
3. KIR 中携带证明的 Loop 与 SLP SIMD；
4. 有界循环展开/版本化与事实驱动专用化；
5. 结构、差分、性能、编译耗时和代码尺寸门禁。

## 固定决策

- SIMD 是自动的。CK 0.12 不增加 source vector type、intrinsic、pragma 或 public
  vector ABI。
- 向量语义位于 KIR，不进入 semantic MIR，也不成为 backend-only side plan。KIR 仍是
  唯一的 target-neutral 优化 IR。
- Vector KIR 使用 fixed-width vector 和 mask。Scalable vector、SVE 和 RVV 不属于
  0.12。
- 现有 baseline 与 native CPU policy 都参与。每份 artifact 只有一个选定 CPU 版本；
  runtime dispatch 和 baseline+feature multiversioning 留到 0.13。
- 被接受的 Native vector candidate 必须 lowering 为真实 SIMD，并由 object disassembly
  证实；不符合条件的 candidate 保持 scalar。C 与 WebAssembly 继续消费 verified KIR，并获得
  循环规范化、展开和专用化，但 0.12 不承诺显式 C 或 WebAssembly SIMD。
- 新增的代码复制与向量 transform 只属于 O3。O0-O2 保持 0.11 pass contract。
- Strict floating point、checked first-error、print/runtime 顺序和 public ABI 行为不变。
  不增加 fast-math mode。
- Contract-sanitizer build 禁用专用化、循环版本化、向量化和展开，保留现有标量管线与
  boundary instrumentation。

## 比较过的架构

### 采用：经过验证的 target-aware Vector KIR

Native TargetMachine 在 KIR 优化前提供规范化的能力与成本 profile。CK 使用自己的事实、
依赖分析、成本模型和独立 certificate checker 生成显式 Vector KIR。LLVM lowering 已验
证的向量操作，并负责 instruction selection、register allocation、scheduling 与 target
legalization。LLVM 目标信息可以描述机器能力或成本，但不能建立 CK alias、range、effect、
bounds 或 safety fact。

### 拒绝：Scalar KIR 加 LLVM vectorization hint

仅使用 loop metadata 实现量较小，但 LLVM 会再次拥有关键合法性与变换。CK 无法为
C/WASM 给出相同解释或 certificate，无法独立拒绝错误向量计划，也无法可靠利用 CK-only
contract fact。

### 拒绝：各后端独立 vectorizer

分别实现 Native、C 和 WebAssembly vectorizer 会复制依赖、版本化和收益逻辑，并可能
对安全性得出不同结论。后端能力应作为同一个 KIR pass manager 的参数化输入。

## 编译架构

0.12 流程为：

    source -> checked program -> semantic MIR
           -> consumer/mode-specific scalar KIR
           -> verified 0.11 scalar pipeline
           -> O3 specialization and canonical loop pipeline
           -> target 支持时生成 verified vector/unroll/SLP KIR
           -> C | WebAssembly | audited Native LLVM

Native 命令必须先创建 host TargetMachine 与规范化 optimization profile，再运行 KIR pass
manager。Optimizer 内部不可见 LLVM object；profile 是普通、确定性的 CK 数据结构。

C、WebAssembly 与默认 inspection profile 报告 vectorization disabled。它们不会收到
vector instruction，因此不需要隐藏的 scalarization pipeline；仍使用同一 KIR 表示、
verifier 和所有有收益的非向量 0.12 transform。

Target-neutral 描述的是 KIR instruction semantics 与 verification，不要求每个 target 获得
完全相同的 optimized graph。Lane count 与 operation selection 由 immutable target profile
参数化，而每个被选 KIR operation 的含义都与 backend 无关。

## Target optimization profile

KirTargetProfile schema 1 只包含：

- consumer 与 target identity，后者表示为 normalized triple，或显式 portable-C/default-
  inspection pseudo-target；
- layout，表示为已知 pointer width 加 endianness，或 portable-unknown-layout；
- CPU identity，表示为 Native policy（baseline 或 native）加 normalized CPU name 与完整排序
  feature string，或 not-applicable；
- legal fixed vector width 与 legal lane type；
- splat、arithmetic、unary、compare、select、cast、insert/extract、load、store 和受支持整数
  reduction 的 operation legality；
- aligned/unaligned memory legality 与整数 cost unit；
- scalar、vector、mask、insert/extract、branch 与 runtime-predicate cost；
- 最大合法 interleave factor；
- 覆盖所有字段，以及在 Native target data 由 LLVM/bridge 生成时覆盖对应 identity 的 digest。

Operation legality 与 cost entry 在适用时以 operation、lane type、lane count、arithmetic
semantics 与 memory alignment class 为 key。Scalar entry 以 operation、scalar type 与
semantics 为 key。Schema 不包含未定义的 catch-all cost。

Native profile 必须拥有已知 layout 与 Native CPU identity。WebAssembly 使用固定 WebAssembly
layout 和 versioned CK scalar cost table。Portable C 与默认 inspection 使用
portable-unknown-layout、not-applicable CPU、versioned CK generic scalar cost table，并且没有
legal vector operation。Unknown layout 禁用包括 address-width predicate 在内的所有 layout-
sensitive transform，不猜测最终 C compiler target。这些 non-Native profile 保持确定性，并在
不依赖 LLVM 的情况下参与同一 profile digest 与 cache identity。

Native baseline 仍为带 mandatory SSE2 的 x86-64，或带 ABI-mandated Advanced SIMD 的
generic ARMv8-A。Native policy 可在精确 host feature set 与 cost profile 证明有利时选择
更宽 fixed vector。更宽不自动等于更好。支持 SVE 的 native AArch64 host 在 0.12 中仍
使用合法 fixed-width profile。

Pinned LLVM bridge 可以把 TargetTransformInfo 与 legalization query 规范化到 profile。每个
被查询 operation 表示为 Legal { cost: u32 } 或 Unavailable；LLVM 返回 unavailable、invalid、
negative 或无法表示的结果时记录 Unavailable，并禁用依赖它的 candidate。缺失 mandatory
schema data、legality/layout data 相互矛盾，或实际生成工作的 cost 为零，会使 profile
malformed 并触发 compiler error。只有明确不产生工作的 no-op（例如 representation-
preserving cast）可以使用零成本。同一 profile digest 进入 cache 与 benchmark identity。

Native profile construction 使用一个带精确 TargetMachine triple/data layout 的 synthetic
LLVM module，以及一个携带该 machine normalized CPU 与排序后 feature attribute 的 internal
probe function。所有 cost 都使用 TCK_RecipThroughput。有限 query domain 是 KIR schema 2
实际可表示的笛卡尔子集：五种 lane type、固定 lane count 2、4、8、16 且总宽度不超过
512 bit、封闭的 arithmetic/
unary/compare/select/cast/insert/extract/reduction operation，以及从一 byte 到 vector byte
width 的 power-of-two alignment class。Bridge 使用各 operation 对应的 TTI arithmetic、
compare/select、cast、memory、vector-instruction、reduction 与 control-flow query，不使用
generic guessed cost。

上述有限全集中的每个 operation key 都必须精确出现一次，值为 Legal 或 Unavailable；target
不支持的 width 记录为 Unavailable，不能省略 key。Mask 使用相同的四种 lane-count candidate。
Probe universe 是 schema 上限，不承诺每个 target 支持每个 width。

由于 `Mask { lanes }` 有意不携带 scalar lane type，mask-only operation cost 统一使用 `i32`
lane tag 作为 canonical schema sentinel。所有非 sentinel 的 `MaskNot` key 都是
`Unavailable`；bridge 针对真实 fixed `i1` mask type 查询 legalization 与 cost，而不是针对
`i32` vector。

每个 entry 还记录 LLVM type-legalization part count 与 legalized type identity。Part count
非法，或 legalized form 被 scalarize/不受支持时，该 operation 为 Unavailable。TTI operation
cost 已包含 target lowering，CK 不会再次乘 legalization part count。Invalid cost、低于零
或高于 u32::MAX 的 valid cost，以及不在 whitelist 内的零都记录为 Unavailable。Canonical
profile byte 使用固定 numeric tag、big-endian integer、length-prefixed UTF-8 string、排序
feature name 与 lexicographic operation key；digest 是 schema tag 加这些 bytes 的 SHA-256。
测试会重复构造 profile；同一 TargetMachine identity 不能生成 byte-identical output 时失败。

## KIR v2 类型与指令模型

Semantic MIR 保持 scalar 与 source ordered。KIR 新增 KirValueType：

- Scalar(MirType)，覆盖所有既有值；
- FixedVector { lane, lanes }，lane 为 i32、i64、u32、u64 或 f64，lanes 为正 u16；
- Mask { lanes }，表示 lane predicate。

Pointer、slice、struct、void 与 source bool 不能成为 vector lane。Mask 不是整数，不能逃
出 internal vectorized region，也不能跨越 public ABI。

KIR instruction result 与 block parameter 使用 KirValueType；function parameter、return value、
call 与 exported storage 保持 scalar MirType。Vector 与 mask value 不能成为 function argument/
result，也不能跨越其 verified vector region 之外的 block edge。

KIR v2 增加封闭的 splat、contiguous load/store、arithmetic、unary、compare、select、
受支持 cast、lane extract 与精确 modular integer reduction 指令族。Vector binary operation 是
既有 Add、Sub、Mul、Div 与 Mod，并受 source type、arithmetic semantics 与 target profile 限制。
其中 f64 Mod 仍然非法，integer Div/Mod 必须同时具有 no-failure proof 和明确 legal target
operation。每个 checked integer vector binary 与 checked integer vector negate 也必须携带
no-failure proof；infallible 或 strict-float operation 不能携带该 proof。Vector unary Neg
遵守既有 numeric semantics；logical mask Not 是唯一 mask unary operation，可表示 comparison
后的 source bool negation，但不会创建 vector bool lane type，也不能携带 no-failure proof。
受支持 vector cast 精确为既有 i32-to-f64 与 u32-to-f64。每个 vector memory operation 记录
region、Memory SSA input/output、lane type/count、byte footprint、known alignment 与 required
alignment。0.12 没有 gather、scatter、masked memory、vector call 或 shuffle。SLP 只允许
source-order identity packing，不生成 lane permutation。

Vector load/store footprint 精确等于映射到其 lane 的 scalar byte 并集。KIR 与 Native lowering
都不得把它扩宽到 slice end、object boundary 或 unmapped page 之外。因此 prefix/tail handling
与 SLP packing 永不授权 speculative over-read 或 over-write。

每个 consumer-specific optimized KIR module 都记录 profile schema 与 digest。Structural 与
certificate verifier 要求提供精确匹配的 profile；默认 inspection module 绑定 target-
independent generic inspection profile，而不是 host profile。

Version predicate 是 high-level、total KIR operation。它可以检查 trip threshold、
divisibility、target-width address interval non-overlap 或 power-of-two alignment。Predicate
求值不解引用内存。地址加法/乘法溢出返回 false 并选择 scalar fallback；绝不回绕成更强
假设。Non-overlap 使用 checked target-width integer address，而不对无关 host-language pointer
做 relational comparison。Zero-byte footprint 是 empty，不形成 end address。

Structural verifier 拒绝 lane count/type 不一致、非法 mask 使用、不支持的 vector
operation、不一致 memory footprint、陈旧 target profile、vector value 逃逸，以及在
vectorization disabled consumer profile 中出现 vector instruction。

## Canonical loop form

现有 CFG/SSA KIR 保持为唯一表示。loop-simplify 为 reducible natural loop 建立经过验证的
canonical form：

- 一个没有 loop-side predecessor 的 preheader；
- 一个 canonical latch 和一条返回 header 的 backedge；
- dedicated exit block；
- 规范化的 induction start、step、comparison 和 trip-count expression；
- loop-closed SSA exit value；
- 显式 LoopId descriptor，包含 parent/depth、blocks、exits、inductions 与 effect summary。

Loop descriptor 是确定性、非权威的分析结果。CFG、inlining、specialization、unrolling 或
vectorization 会使受影响 descriptor 及其依赖 fact 失效。Pass manager 必须重建 dominance、
Memory SSA、contract-instance mapping 与 loop descriptor，消费者才能复用它们。

Irreducible loop、无法在不改变 effect 的情况下规范化的 multiple latch、non-affine
induction 或预算耗尽仍是合法 scalar KIR，并产生稳定保守解释。

## 循环访问与依赖合法性

符合条件的内存访问必须对一个 canonical induction 连续且 affine：

    byte_address(iteration) = base + element_size * (a * iteration + b)

target-width overflow 必须静态证明不可能，或由 overflow 时返回 false 的 version predicate
覆盖。首版要求正向 unit-stride（a = 1）vector group；负向或其他 affine stride 可帮助证明
disjointness，但不会被 vectorize，也不会生成 gather/scatter。

分析按以下顺序使用：

1. 既有 region partition、Memory SSA、noalias、readonly/writeonly、alignment、slice
   interval、range 与 effect fact；
2. 精确 same-base offset/distance reasoning 与保守 integer affine dependence test；
3. cloned fast path 上可选的 runtime non-overlap/alignment/trip predicate。

每个潜在 loop-carried read/write pair 被分类为 independent、已证明受支持的 reduction、
dependent 或 unknown。Dependent pair 阻止向量化。Unknown write/write 或 read/write
dependence 也会阻止向量化，除非一组允许的 runtime predicate 合取完整证明 footprint 不
重叠。Read/read pair 不构成顺序依赖。

Call、runtime operation、print、volatile-like effect、unknown memory 与 ordered failure 会
阻止跨迭代重排，除非之前的 verified transform 删除调用/效果或证明其无关。Vectorizer
绝不从不同 source name 或 raw pointer 虚构 noalias。

## 循环版本化

一个循环最多拥有两条路径：

- 完整假设已检查或静态证明的一个 SIMD fast path；
- 保持不变的原始 scalar loop fallback。

Transform 保留原 scalar block，而不是重新构造一个等价循环。所有 runtime predicate 在
第一个原循环 effect 前执行。一个 loop version 最多包含四个合取 atomic predicate，不
允许析取。Predicate 为 false、地址溢出、trip count 不足、misalignment 或 overlap 都选择
scalar path。

Fast path 在 scalar epilogue 前必须至少执行两个完整 vector；收益模型可以提高该阈值。
Tail iteration 按原顺序在 scalar epilogue 执行。0.12 不剥离 scalar alignment prefix：它只能
使用 profile-legal unaligned operation、证明 alignment、增加 alignment predicate，或拒绝
candidate。Versioning 不得抑制 empty loop、移动可观察 effect，或改变哪个 checked
operation 报告 first error。

## 提案与独立验证

Analysis 和 cost model 只提出 transform，不授权它们。Append-only proof language 增加以下
封闭步骤：

- canonical loop 与精确 trip partition；
- induction 与 affine access mapping；
- static alias/dependence classification；
- runtime predicate completeness 与 false-on-overflow 行为；
- lane-to-scalar iteration mapping；
- vector operation equivalence 与 memory footprint；
- 精确 arithmetic semantics 下的 reduction associativity；
- scalar fallback identity 与 epilogue coverage；
- specialization fact scope 与 clone argument mapping；
- target-operation legality、cost decomposition、code growth 与 budget accounting。

VectorizationPlan 记录 input LoopId、VF、UF、scalar-to-vector map、memory group、可选
predicate、epilogue、target-profile digest、estimated cost、code growth 与 proof root。SLP
与 specialization 使用相应的封闭 record。

Pass manager 保持两个相互分离的状态层。KirVerifiedProgramState 拥有 module、contract
fact、proof arena、eliminated guard、verification cache、evidence generation 与确定性 IR ID
allocator。只有该状态会被 speculative transaction 复制，并且只能整体 commit 或 rollback。
KirOptimizationAuditState 拥有冻结的 proposer/checker budget ledger、单调 attempt sequence、
accepted/rejected counter、stable explanation 与 budget fallback。Audit state 只能 append/debit，
绝不随 KIR rollback。

Verification 与 analysis cache 只能在精确 structural identity 下缩短耗时。Immutable target
profile 会缓存完整 validation 结果；copy-on-write mutation 必须清除该结果。Changed pass 后，
每个发生变化的 function 都完整重验，精确未变化的 function 可复用既有 structural verdict，
同时仍执行 module-wide function/block/instruction/value/region/memory identity uniqueness 与
完整 fact/proof/rewrite verifier。Dominance 只能按其实际依赖的 ordered block/successor CFG
复用，并扣除完全相同的确定性 analysis budget。仅用于 discovery 的 loop descriptor 可以省略
cryptographic CFG digest，但任何已 materialize 的 proposal 或 certificate 必须重新计算完整
digest。若 discovery 已证明 specialization、vector/SLP 与 unroll candidate set 都为空，pass
manager 可直接记录 verified no-op stage，而不分配 speculative program-state copy。No-op
frontier 只有在中间 pass 未改变 induction structure，且每组缓存 descriptor 仍保留精确 function
identity 时，才能复用前序 discovery-only loop descriptor；否则必须重算 analysis。以上规则均
不得改变 candidate、checker、budget、cost、profitability 或 benchmark threshold。

Proposal 与 checker step 在执行时直接扣 outer audit ledger；rejection、复用 specialization 或
未获胜 frontier candidate 都不退款。Audit record 使用 transaction 前稳定的 source/KIR
identity、kind、VF 与 UF 标识 candidate，绝不引用 trial-only ID。Candidate 按 function 与
pipeline stage 顺序和以下唯一 stable key 枚举：specialization 使用 caller FunctionId、call
InstructionId、callee FunctionId 与 canonical fact-set digest；loop frontier 使用 FunctionId、
LoopId、kind rank Loop SIMD/full unroll/partial unroll、scalar-or-SLP variant rank，再按递增
VF/UF；residual SLP 使用 FunctionId、BlockId、root InstructionId，再按递增 lane count。每个
stage 完成后才进入下一 stage，共享 ledger 永不重置。接受时只交换 verified program-state
snapshot 并 append accepted audit record；拒绝时只丢弃 snapshot 并 append stable reason。

Independent checker 读取 pre-transform KIR、proposed record、target profile、facts 与 proofs。
它不调用 vectorizer、dependence analyzer 或 proposer cost model，也不把它们的结论当作
premise；它根据封闭 record 重新计算 legality、integer cost total、profitability threshold、
structural growth 与 budget consumption。只有 checker 接受完整 proposal 后才提交 transform，
然后照常运行 post-transform structural/evidence verifier。

Commit 前预算耗尽会丢弃整个 proposal 并保留 scalar code。Malformed/false certificate 或
post-commit verification failure 是 compiler error，并阻止生成所有 artifact。

## Loop SIMD

Loop vectorizer 首版接受 innermost canonical loop：trip expression 可计数、unit-stride
memory、fixed vector 对目标合法且没有未解决 ordered effect。支持 lane-wise i32/i64/u32/
u64/f64 arithmetic、compare、受支持 cast、mask select、contiguous load/store 与 splat。

Strict f64 operation 保持为独立指令，逐元素 rounding 与原程序相同。禁止 FMA contraction
和跨 lane reassociation。纯 element-wise f64 loop 可向量化；floating reduction 不可。

Eligible loop 内单个 side-effect-free diamond 只有在两条 arm 立即 reconverge、定义相同
scalar result，且不含 memory access、guard、call、runtime effect 或 certificate-scoped
operation 时，才能 if-convert 为 compare、mask 与 select。其他 control predication 保持
scalar。

当 target 支持且 checker 证明精确 lane partition 与 horizontal fold 时，可向量化 unchecked
modular integer add/mul reduction。Checked integer reduction 在 0.12 中保持 scalar。其他
reduction、scan、recurrence、gather/scatter、complex predication 与 interleaved memory 不
属于范围。

这里的 supported source surface 不承诺每个 kernel 在每个 target 都会被接受。每个具体
target profile 仍必须证明未改变的 20% profitability threshold。在固定 baseline profile
中，AArch64 可以接受完整结构 corpus，而 x86-64 会保守地让高吞吐 strict-f64 division 保持
scalar，并让 modular integer add/multiply reduction 以 scalar KIR 交给 Native LLVM loop
vectorizer。后者是 target-specific lowering 选择：fixed-vector KIR plan 会在每个 chunk 执行
horizontal fold，而固定 x86 backend 会保留 vector accumulator 并仅在 loop exit fold 一次。
跨 target 测试必须断言精确的 accepted subset 与稳定 fallback；每个实际 accepted plan 都必须
提供结构 KIR vector 证据。使用 x86 Native lowering fallback 的 benchmark 还必须通过 pinned
object disassembly 单独证明 SIMD，但该证据不能宣称其 KIR plan 已被接受。

对于 checked element-wise operation，只有既有 fact 或允许的 version predicate 证明每个
vector lane 都不会失败时，fast path 才合法。Scalar fallback 保留所有原始 guard。不实现
vector failure 的 per-lane recovery。

## SLP SIMD

SLP 在 scalar full-unroll opportunity 之后、final cleanup 之前处理 straight-line scalar DAG。
它打包 lane type 与 arithmetic semantics 相同的同构、独立 operation。Memory pack 必须连续，
并与 Memory SSA 顺序一致。Packing 不得跨越 guard、call、runtime/print effect、unknown write、
block boundary 或 certificate dependency。

首版 SLP 支持 splat、lane-wise arithmetic、comparison、cast、select 与 contiguous load/store。
它不做 speculative predication、arbitrary shuffle synthesis、horizontal f64 operation 或
partial vector call。被拒绝的 pack 保持全部 scalar instruction 不变。

同一 function、block 与 root 上相互重叠的 residual SLP alternative 必须基于同一个 immutable
pre-state，按稳定递增 key 提案并独立检查。Winner 依次选择绝对 modeled cost reduction 最大、
transformed cost 更低、code shape 更小、stable key 更小的有效方案。其余有效方案仍计费并记录为
non-winner，且只能提交一个 winner，避免先出现的窄 pack 破坏收益更高的宽 pack。

## 受控展开

Unrolling 必须确定且由成本驱动：

- vector interleave/unroll factor 只能是 1、2 或 4，且不得超过 target profile 上限；
- constant-trip scalar loop 只有在 trip count 不超过 8、原 body 不超过 16 个 KIR
  instruction unit 且满足公共 code-growth budget 时才能 full unroll；
- 其他 scalar partial unrolling 只能使用 factor 2 或 4，并且必须独立消除足够 branch cost，
  或由 trial unroll 加 SLP plan 共同达到收益门槛。由 SLP 证明收益的 unroll 与对应 pack 构成
  一个经过独立检查的事务，任一半都不能单独提交。

每个 scalar full-unroll、scalar partial-unroll 与组合 unroll-plus-SLP proposal 都必须相对
同一冻结 pre-state 预测至少 10% total loop execution-cost reduction，且至少节省两个 absolute
cost unit。这些要求叠加于上述 trip、body、factor 与 growth 限制。Loop SIMD 保持更严格的
20% 门槛。Frontier proposal 先通过自身门槛与 independent checker，只有已接受 proposal
才能进入公共 winner 比较。

Checker 证明 iteration coverage、order-sensitive effect preservation、phi/LCSSA mapping 与
精确 remainder behavior。Unrolling 绝不越过 scalar program 原本可能停止的位置复制潜在
可观察 failure 或 call。

## 事实驱动函数专用化

O3 可以基于一组规范化 dominating fact 克隆 internal direct-call target：精确 scalar/bool
constant、精确 slice length、已证明 alignment、完整 noalias relationship，以及 readonly/
writeonly/effect summary。Trusted-contract fact 保留 call-instance scope，只能专用化它授权
的 dominated instance。

Export name、signature、thunk 与 public ABI 绝不改变，并保留 generic body。Clone name 是
internal deterministic digest，不能 export。0.12 不专用化 recursive SCC、indirect call、
address-taken function、runtime call 与 sanitizer mode。

Specialization 在 O1 fact/check prefix 之后、O2 inlining 之前执行，使 clone 能暴露 constant
folding、check elimination、loop bound 和后续 vectorization。每个 trial 拥有
KirVerifiedProgramState 的完整副本：module、contract fact、proof arena、eliminated-guard
record、verification cache、evidence generation 与确定性 IR ID allocator；其工作直接计入
outer KirOptimizationAuditState。它替换 scoped fact、创建并重定向 clone，只执行有界
clone-local CFG/SCCP/range/check/DCE scalar
finalization。Trial 期间禁用 nested specialization 与 interprocedural inlining。

Specialization checker 针对 copied pre-state 独立验证 fact scope、argument/ID mapping、scalar
cost reduction、code growth 与 caller/callee 两侧预算扣除。接受必须由已经 materialize 的
scalar reduction 满足正常 10% 且两个 unit 的 specialization 门槛；预测但尚未提交的 vector
收益不能让原本无收益的 clone 通过。接受时完整 verified program-state 副本原子替换
pre-state；拒绝时只丢弃该副本，audit charge 与 rejection explanation 保留。已接受 clone
随后只遍历一次正常 O2 与剩余 O3 loop/vector pipeline，不迁移
trial proof/descriptor，也不会成为第二个 specialization root。

Specialization clone 自身永不成为 specialization root。相同 canonical fact set 复用同一
digest-named clone，限制只计算不同 fact set。即使 clone 被复用或拒绝，pass manager 仍扣除
trial work。

每个 original function 最多有三个 specialized clone，每个 module 最多 24 个。专用化
instruction-growth allowance 为：

    max(64, min(4096, ceil(pre-specialization module KIR units / 4)))

超过共享 allowance 的 clone 不得提交。

## 确定性收益与预算

Cost 使用非负整数 unit。模型将 target-profile operation cost 与 CK-owned trip range、
alignment、alias、effect、vector setup、runtime predicate、scalar epilogue 和 code-growth
cost 组合。它不使用 wall clock、unordered iteration、machine load 或 profile feedback。

Loop candidate 使用精确或保守 trip estimate 比较 scalar iteration cost 与 vector body、
predicate、epilogue cost。未知 trip count 增加 runtime threshold，至少为 computed break-even
与 2 * VF。Vector loop 必须预测至少 20% execution cost reduction。SLP pack 或
specialization 必须预测至少 10% local reduction 且至少节省两个 absolute cost unit。相同
cost 时依次选择更小 code shape、更低 VF/UF、source/KIR identity order。

固定结构限制为：

- 每个 loop 最多一个 SIMD version 加一个 scalar fallback；
- 每个 loop 最多四个 runtime predicate；
- 最大 unroll factor 为 4；
- transformed loop instruction unit 不超过原 loop 的三倍加 32 个 control unit；
- 使用上述 specialization 限制；
- post-0.12 O3 KIR aggregate instruction unit 不超过 specialization 前 KIR 的两倍。

同一 function 内所有 0.12 specialization、Loop SIMD、SLP、versioning 与 unroll proposal
work 共享 64 * pre_transform_function_kir_units + 128 步；其 independent checker 共享
96 * pre_transform_function_kir_units + 256 步。pre_transform_function_kir_units 固定为该
original function 进入 O3 时的值；clone 使用其 original function 的冻结值。Specialization
trial 同时扣除 caller 与 original callee budget；拒绝或复用 clone 都不重置预算。算术使用
saturating u32。预算耗尽是带稳定原因、无部分修改的保守拒绝。

## O3 管线顺序

O0、O1、O2 保持 0.11 顺序。O3 依次执行：

1. O1 CFG/SCCP/range/check prefix；
2. fact-driven direct-call specialization 与隔离的 complete-state scalar trial finalization，随后刷新
   CFG/SCCP/check；
3. 既有 O2 inline、Memory SSA、GVN、forwarding、DSE、propagation 与 check cleanup；
4. loop-simplify 与 canonical descriptor verification；
5. natural-loop/induction analysis、LICM、induction simplification 与 scalar
   propagation/check cleanup；
6. 重建 canonical loop、dependence、Memory SSA 与 cost descriptor，并为每个 innermost loop
   冻结同一个 immutable scalar pre-state；
7. 从同一 pre-state 分别提出 Loop SIMD（包含可选 versioning 与 target-bounded VF/UF）、
   small constant full-unroll 与 scalar partial-unroll alternative，后两者各自可为 scalar-only
   或原子追加 SLP；独立验证
   每个 proposal，按 predicted total cost、code shape、更低 VF/UF、KIR identity 比较已接受
   alternative，并为每个 loop 至多事务提交一个 winner；non-Native profile 只提出 scalar-
   unroll alternative；
8. 在选定 loop transaction 后重建 descriptor；
9. 对已提交 loop region 之外执行 residual Native SLP planning、独立验证与事务 rewrite；
10. 对剩余 scalar value 执行 final SCCP、DCE、Memory SSA cleanup、evidence validation 与
    structural verification。

每个 named pass 记录 changed/verified state。所有 CFG-changing step 显式声明 preserved
analysis；其余分析全部失效并重建。

Optimization statistic 增加 canonicalized/versioned/vectorized loop count、SLP pack 与 vector-
operation count、scalar epilogue、各 unroll factor、specialized clone count、按 stable reason
分类的 rejected-candidate count，以及所有 analysis-budget fallback。计数使用确定性 KIR
identity order。

## 后端契约

### Native LLVM

Verified fixed vector 结构化 lowering 为 LLVM fixed vector type 与 operation。Mask lowering
为 target-legal predicate form。只有 profile 标记 legal 时才生成 unaligned access；alignment
attribute 不能强于 verified alignment。Strict f64 禁止 contraction/reassociation。CK loop/
vector fact 与现有 alias、range、alignment strengthening 一样，在 LLVM optimization 前审计。

Bridge ABI 因新增 normalized target cost/capability query 与 vector construction operation 而
升级。除非独立评审的 toolchain change 证明等价契约，0.12 继续固定 LLVM 22.1.8。

在 x86-64 MSVC 上，LLVM 会为任何包含 floating-point operation 的 module 生成未定义
`_fltused` 引用。因此每个 CK 生成的 COFF module 都拥有一个不导出的 `weak_odr`/
COMDAT-any 零定义，embedded freestanding runtime 也保留等价 `selectany` 副本；两种 object
共同链接时会合并，而只含 floating-point 的 Native library 无需 CRT 或 runtime object 即可
链接。这属于 compiler-support closure，不是 public CK symbol，也不增加 Runtime ABI。

### C 与 WebAssembly

它们的 0.12 target profile 禁用 Vector KIR，继续消费 verified scalar KIR，并可获得
specialization、canonical-loop cleanup 与 controlled scalar unrolling。Generated C 保持
portable standard C；WASM 不会静默要求 SIMD128。任一后端的显式 SIMD 都需要单独的
versioned design。

## 解释、检查与 fallback

--explain-optimization 的确定性输出增加 candidate kind、LoopId 或 pack/call identity、
selected/rejected status、VF/UF、predicate、estimated scalar/vector cost、code growth、proof
root 与一个稳定 reason。必须包含的拒绝原因有 unsupported consumer/target、sanitizer mode、
irreducible/noncanonical loop、unknown trip、unresolved dependence、unsupported effect、strict-
float reduction、illegal target operation、profitability threshold、code-size budget 与 analysis
budget。

emit-kir 保持既有默认 inspection 行为，即 scalar 且 target independent。它增加 --consumer
inspection|c|wasm|native-library|native-executable，默认 inspection。--cpu baseline|native
只对两个 Native consumer 合法，且缺省为 baseline。Native consumer 要求 compiler 启用
native-toolchain feature；native-executable 要求存在 executable build/run 接受的同一合法
main entry。该命令打印对应 consumer/profile 的精确 final KIR，不根据源码内容推断 artifact
kind。emit-llvm 使用 Native baseline；build 使用所选 CPU policy；run 与现有行为一样使用
native。

Unsupported candidate 与 analysis budget 是正常保守 fallback。Invalid target identity、
invalid certificate、stale evidence 或 invalid post-transform KIR 是 compiler error，不生成
partial output 或 cache entry。

## 兼容性、ABI 与 cache identity

CK source syntax、type system、semantic MIR、diagnostic、strict f64、checked status/first-error
rule、slice ABI、public symbol 与 Native C ABI version 1 保持不变。Runtime ABI 保持 version 2，
因为没有新增 runtime helper。

KIR print/schema identity 从 kir-v1 升级到 kir-v2；0.12 不新增持久化 serialized KIR artifact
cache。既有 Native object/run cache 升级为 entry magic CKCOBJ02、key schema 3 与 manifest
schema 3。其 key 与 manifest 除既有 identity 外，还覆盖完整 target-profile digest、vector
cost-model schema、vector proof schema 与所有固定预算常量。Private LLVM bridge ABI 从 2
升级到 3。Compiler 与 package version 只在实施时变为 0.12.0。0.11 object/cache entry 会在
entry 或 identity 检查中失败，不能被 0.12 接受。

Vector 与 specialization clone symbol 仅为 internal，并从 header、export、dynamic symbol
table 与 public ABI audit 中排除。

## 验证策略

验收必须包含以下全部项目，不得 ignore test 或降低门槛：

1. 所有 0.11 language、ABI、CLI、artifact、runtime、sanitizer、differential、mutation、
   performance 与 six-host contract 继续全绿。
2. Unit/mutation test 覆盖 loop normalization、LCSSA、trip partition、affine overflow、
   dependence distance、runtime predicate completeness、lane map、mask、vector memory
   footprint、reduction、fallback identity、unroll coverage、clone fact scope、target
   illegality、stale profile/proof、forged cost/growth record 与 atomic budget fallback。
3. Generated differential kernel 在 zero、short、exact-vector、remainder、maximum-safe、
   overlapping、disjoint、aligned、misaligned、checked 与 unchecked input 上对比 O0 scalar
   semantics 与 O3 结果。
4. Adversarial case 对 irreducible control flow、unknown write dependence、call/effect、
   strict f64 reduction、possible first error、overflowing address predicate 与 over-budget
   module 保留 scalar execution。
5. KIR 与 pre-LLVM structural test 证明每个 accepted fixed-vector KIR plan 都含预期 vector
   operation。x86-64/AArch64 上的 pinned object disassembly 证明存在真实 SIMD instruction。
   显式命名的 x86 horizontal-reduction fallback 只作为 Native lowering path 验收，不能作为
   fixed-vector KIR plan 已被接受的证据。
6. 所有受支持 host 上的 baseline/native CPU policy 都执行 correctness 与 feature-containment
   test。Native machine code 只能使用 resolved feature string；baseline artifact 不得使用
   optional ISA feature。
7. 精确 final candidate SHA 通过 required ten-job matrix：quality、Native integration、六个
   host target 与 x86-64/AArch64 performance。

## 性能与尺寸契约

Strict Native performance report 从 schema 6 升级为 schema 7，并固定 candidate version
0.12.0。它固定 compiler、source、target profile、cost-model/proof schema、CK artifact、
oracle artifact、sampling schedule 与每个 source digest。Pinned 0.11 replay compiler 是
commit 80c0acf6bb5d65e4d9d40352b9501ea32b79f43d。其独立构建的 compiler、Native artifact、
fixed independent C oracle、recipe 与 digest 和既有 0.10 replay bundle 一样保留。

Scalar-regression protocol 为 rotating-twelve-channel-v1。Channel 顺序精确为
candidateNativeUnchecked、candidateNativeChecked、currentClangUnchecked、
currentClangChecked、replayV011NativeUnchecked、replayV011NativeChecked、
replayV011ClangUnchecked、replayV011ClangChecked、replayV010NativeUnchecked、
replayV010NativeChecked、replayV010ClangUnchecked 与 replayV010ClangChecked。Warm-up 三行，
sample 二十行；第 r 行为 [(r + i) % 12 | i in {0, ..., 11}]。所有 stream 在同一进程、相同输入
上执行，并保留既有每 sample 七次调用与固定 batch identity。Schema 7 记录每个实际 order、
sample、upper median、artifact digest 与 result；缺失 stream 不得回退到历史数值。

测量在稳定 x86-64/AArch64 worker 上使用 portable baseline policy；除非另行批准固定硬件
identity，native-policy measurement 仅作诊断。

Hand-written SIMD C 使用 pinned Clang 22.1.8 构建，hand-written SIMD Rust 使用 pinned Rust
1.90.0 构建；两者都使用 architecture-specific baseline flag，禁用 fast math/contraction，
并且不得使用 CK baseline profile 不具备的 CPU feature。两份 implementation 都必须在固定、
已声明的 valid input domain 上通过 differential 与 undefined-behavior audit。Manifest 必须列出
所有 precondition；input 只能依据 pinned manifest 排除，绝不能在测量后排除。缺失或无效的
C/Rust artifact 会使 gate 失败，不能通过移除该 competitor 处理。

Vector 与 domain-fact runtime gate 对 checked 和 unchecked CK 分别使用
rotating-three-channel-v1：candidate CK、pinned C 与 pinned Rust。每轮有三个 warm-up row 和
二十个 sample row；第 r 行按 r % 3 轮换三个 channel。所有 channel 在同一进程、相同输入上
运行，使用相同的每 sample 七次调用、固定 batch identity 与 upper-median statistic，并记录
每个实际 order 与 sample。Generic domain-fact gate 使用 pinned generic Clang/Rust artifact
替换 hand-SIMD artifact。每个 dynamic library 只在 correctness、warm-up 与 sampled batch 之前
打开一次并解析一次 typed kernel entry；计时 batch 只能调用缓存入口，dynamic symbol lookup 与
逐调用 string dispatch 必须位于计时区外。

固定四元素 `slp_quad` microkernel 的每个 timed sample 之前，三个 channel 都执行一次相同的
unmeasured batch。该 short-kernel conditioning 写入固定 manifest，并同等应用于 CK、C 与
Rust；它不改变三个 warm-up row、二十个 timed row、每 sample 七次 timed call、batch
identity、order、statistic 或 threshold。

Release gate 为累积门槛：

- 所有既有 0.11 Native/Clang、0.11/0.10 replay、checked/unchecked 与 optimizer-latency
  limit 保持不变；
- 对每个经过审计的 vector-eligible kernel，oracle 是其 fixed hand-written C+SIMD 与
  Rust+SIMD 实现中较快的有效 median；分别在 checked 与 unchecked mode 下，CK throughput
  在 x86-64 与 AArch64 上至少达到这些逐 kernel oracle geometric mean 的 95%，每个 kernel
  至少达到其 oracle 的 90%；
- 在另一套 CK contract 暴露而 fixed generic source 不含这些约束的 domain-fact suite 中，
  每个 kernel 的 generic oracle 是 pinned Clang O3 与 Rust O3 中较快的有效 median；CK 在每个
  架构上至少超过这些 oracle 的 geometric mean 5%；
- hand-written SIMD oracle 获得其 source language 可表达的全部等价 precondition，并保持
  CK strict-float 与 safety semantics；尤其 integer conversion oracle 必须实现完整 CK `u32`
  domain，不能依赖 benchmark corpus 的数值恰好落在 `i32` 范围；
- unchanged scalar regression corpus 相对 independently replayed pinned 0.11 compiler 的
  geometric mean 最多慢 3%，单项最多慢 8%；
- Native artifact-size suite 相对 pinned 0.11 compiler aggregate 增长不超过 35%，单项不
  超过 2.5 倍；
- 对 baseline O3 source-to-relocatable-object compilation，candidate/replayed-0.11 time ratio
  的 geometric mean 不超过 1.5，且任何单项 ratio 不超过 2；
- KIR analysis/optimization 继续满足相对固定 0.10 MIR optimizer 的 suite-median 2x 与
  individual 3x limit。

Artifact-size corpus 使用精确相同 source，为两种 safety mode 生成成对的 baseline Native
relocatable object。Size 是 archive/link 之前的精确 object byte length；排除 cache、debug
sidecar 与 distribution container。Source-to-object compile-time corpus 使用相同 source/mode
pair、全新 output path、禁用 artifact cache、三个 warm-up pair 与十五个 measured pair。
Candidate-first 与 replay-first 顺序交替，并报告已终止子进程 user+system CPU time 的 upper
median；它排除托管 worker 被调度移出的时间，但保留全部编译器工作。缺失、失败或不匹配的
object 会使两项 gate 失败。

Vector corpus 至少包含 contiguous map/zip、strict element-wise f64、integer transform、
target legal 时的 exact modular integer reduction、来自小型 unrolled body 的 SLP、runtime
noalias versioning，以及暴露 fixed slice length 的 specialization。Memory-bound 与 compute-
bound case 都是必需的。修改 source、compiler identity、threshold、statistic、target profile
或 exclusion rule 属于需要评审的契约变更，不得为使候选通过而修改。

## 完成条件与未来边界

只有精确 candidate SHA 满足上述全部 semantic、structural、performance、compile-time、
size、cache 与 six-host gate，且当前英文/中文文档与实现一致时，CK 0.12 才算完成。

以下内容不属于 0.12：source SIMD type/intrinsic、fast math、floating reduction
reassociation、gather/scatter、arbitrary shuffle synthesis、masked fault recovery、complex
loop predication、scalable vector、GPU target、cross-compilation、public JIT API、profile
feedback、runtime CPU dispatch 与 Auto-Tuning。Baseline+feature multiversioning 与 PGO 留到
0.13，offline Auto-Tuning 留到 0.14。
