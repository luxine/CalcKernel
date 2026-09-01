# CK 0.12 设计第一轮对抗性审查

日期：2026-09-01
审查对象：`specs/0.12/fact-driven-vector-optimizer.md` 与中文镜像
基线：`6c00b8044fe2e9179726fc2730951403822e7468`
结论：**暂不通过；4 个阻断项，1 个必须随修订澄清的重大项。**

本轮只检查会阻断实现、破坏语义闭环或让验收不可复现的问题，不把实现规模、
一般性扩展建议或文字偏好列为问题。仓库锁定依赖的非 Native 基线构建和 484 项
测试全部通过，因此以下结论针对 0.12 增量设计，而不是既有基线故障。

## 阻断项 B1：专用化 trial 与正式 O3 流水线重复且事务边界不闭合

设计要求专用化 trial 在 O2 前运行函数内 CFG/SCCP/range/check、loop、unroll、
Vector/SLP，并把“已经优化的 clone”提交；随后正常 O2/O3 又会在该 clone 上运行
相同类别的分析和变换。现有 pass manager 的证明、已删除 guard、contract facts、
verification cache 和统计属于模块级事务状态，并没有可直接分叉后再可靠合并的
函数局部证据 arena。设计也未规定 trial 中生成的 `ValueId`/`InstructionId`、
`ProofId`、Memory SSA generation、LoopId、预算消耗和 rejection explanations 如何
原子迁移到主模块。

这不只是实施细节：重复向量化可能形成嵌套 version、重复 unroll 或让正式流水线
消费 trial 后已失效的描述符；若不迁移 trial 证据，则提交的优化 clone 不能通过
独立验证；若迁移，则当前设计没有定义 ID 重映射和证据 generation。必须把专用化
与下游变换定义成一个不会重复运行、可独立验证并能原子提交的闭合事务。

## 阻断项 B2：部分展开加 SLP 先于 Loop SIMD，会抢占主循环候选

O3 顺序第 7 步先执行“Native combined scalar partial-unroll-plus-SLP”，第 8 步才做
Loop SIMD。第 7 步一旦提交，就会改变循环体、Memory SSA、成本和 descriptor，甚至
引入 Vector KIR；第 8 步看到的已不再是原始 canonical scalar loop。设计没有要求
第 7 步只处理已被 Loop SIMD 以稳定理由拒绝的循环，也没有定义二者的联合候选比较。

因此同一循环可能先接受局部收益 10% 的 SLP 方案，却失去本可满足 20% 门槛、整体
收益更高的 Loop SIMD 方案。这违反设计自身“更小 shape/更低 VF 只在成本相同后
择优”的确定性盈利目标。必须让 Loop SIMD 先获得原始 scalar loop 的候选权，或让
两种方案在同一未变 pre-state 上比较后只提交全局更优者。

## 阻断项 B3：`emit-kir --consumer native` 不能唯一映射现有 consumer 契约

现有 `KirConsumer` 区分 `NativeLibrary` 与 `NativeExecutable`；这一区分决定入口 wrapper
和 Native lowering 行为。设计同时要求 target profile digest 覆盖 consumer identity，
但新增 CLI 只给出单值 `native`。对于含 `main`、不含 `main`、既可作为 library 又可
作为 executable 的源码，无法从源码内容无歧义推导所需 consumer；自动推断还会让
同一源码的 inspection KIR 与实际 `build --kind` 不一致。

必须让 CLI 显式选择 `native-library` 或 `native-executable`（或者给出另一个完全确定、
与 build/run 一致的映射规则），并明确 `--cpu` 的合法组合。

## 阻断项 B4：LLVM TTI 到整数成本表的归一化规则不足以复现

设计固定 LLVM 22.1.8 并要求 profile 的每个 emitted operation 都有非负整数 cost，
但没有固定 TTI cost kind、synthetic query IR、`InstructionCost` 的 invalid/unknown
处理、legalization multiplier，以及如何把 scalar/vector/memory/predicate 查询统一到
同一整数单位。LLVM 的 TTI 成本查询依赖具体 IR type、data layout、function/target
attributes 和 cost kind；仅写“normalize TTI and legalization queries”会允许多个都
看似合规却产生不同 VF/UF 和 profile digest 的实现。

必须固定有限查询域、synthetic probe module/function、`TCK_RecipThroughput`（或另一个
明确 cost kind）、legalization 与 cost 合并规则、饱和/Unavailable 规则，以及 digest
序列化顺序。否则 checker 无法独立重算 proposer 的 target cost，六主机结果也不可
复现。

## 重大项 M1：`KIR/cache v2` 混合了不存在的序列化 KIR cache 与现有 object cache

仓库当前有打印 KIR contract 和 Native run/object cache，但没有可读取的持久化 KIR
artifact cache。设计写“KIR/cache contract advances from kir-v1 to kir-v2”，容易被
实现成只改一个字符串，也可能被误解为要新增 KIR 序列化缓存；两者工作量与兼容面
完全不同。修订必须明确 0.12 是升级 KIR schema/print identity 与既有 Native object/run
cache key/manifest，还是确实新增 KIR artifact cache。若不新增，应明确禁止旧 object
cache 命中，并给 manifest 自身的版本策略。

## 已检查且不构成阻断的事项

- Vector KIR 不跨公开 ABI、Native ABI 1 与 Runtime ABI 2 不变，和当前仓库的 ABI
  分层一致。
- C/Wasm 使用 scalar profile 而不承诺 SIMD，与未知 C target layout 的保守策略一致。
- checked first-error 通过不失败证明、原始 scalar fallback 和原序 epilogue保持，语义
  路径闭合；后续实现仍须覆盖 mutation/differential tests。
- v0.11 replay commit、LLVM 22.1.8 与 Rust 1.90.0 均能在仓库现有 pin/CI 中找到来源，
  性能门槛虽严格但不是逻辑矛盾。
- 远程 CI 现有 workflow 支持手工 dispatch；feature branch push 本身不自动触发，
  执行计划需显式使用 workflow dispatch，并以 exact SHA 核验，不需要修改设计目标。

## 第一轮判定

在 B1-B4 未经复诊和修订前，设计不能进入实施计划拆分。M1 应与阻断项修订同步解决，
避免计划基于不存在的缓存层。下一步必须先用现有 pass manager、CLI、bridge 和 cache
实现复诊上述结论，再修订双语设计。
