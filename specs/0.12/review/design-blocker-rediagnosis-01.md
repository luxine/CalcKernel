# CK 0.12 第一轮阻断项复诊

日期：2026-09-01
输入：`design-adversarial-review-01.md`
结论：**B1-B4 全部复诊成立；M1 成立并须同步修订。**

## B1 复诊：成立

仓库的 `KirPassManagerResult` 持有模块级 `ProofArena`、guard elimination、contract facts、
explanations、statistics 与完整 `VerifiedKirState`。`ProofArena` 只有单一 generation，证明
步骤直接引用模块内的 `FunctionId`、`BlockId`、`ValueId` 和 `InstructionId`。当前没有把
一个函数局部 trial 的证据安全合并回主 arena 的接口，也没有跨模块 ID 重映射契约。

因此不能把“函数局部、已经完成 loop/vector 优化的 clone”直接塞回主状态。修订采用
完整 `KirOptimizationState` 快照作为 transaction：trial 拷贝模块及全部证据/事实/
allocator/budget 状态，失败整体丢弃，成功整体替换，避免局部证据合并。同时 trial
只做事实替换及有界 scalar finalization，不提前提交 loop/unroll/vector/SLP；所有 clone
与普通函数只通过一次正式 O2/O3 后半流水线。

## B2 复诊：成立

现有 O3 在 O2 后才构建 natural-loop analysis；任何 CFG 或 loop-body mutation 都要求
重建 dominance、Memory SSA 和循环分析。若 scalar partial-unroll-plus-SLP 先提交，后续
Loop SIMD 只能在已改变的 pre-state 上重新分析，不能再与原始循环的方案公平比较。

修订采用同一 immutable canonical scalar-loop pre-state 的候选 frontier：Loop SIMD、
constant full-unroll(+SLP) 与 partial-unroll(+SLP) 分别提案和独立验算，再按总预测成本、
code shape、VF/UF 和 KIR identity 选择至多一个方案提交。非 Native consumer 只生成
scalar-unroll 候选。这样不依赖 pass 先后偶然抢占。

## B3 复诊：成立

`KirConsumer` 当前恰有 `C`、`WebAssembly`、`NativeLibrary`、`NativeExecutable`、
`Inspection` 五个值，Native 两种 consumer 已影响 entry wrapper 与 lowering。单一
`native` 无法保持 profile digest 的 consumer identity，也无法保证 inspection KIR 与
对应 build/run 路径一致。

修订让 `emit-kir --consumer` 精确使用
`inspection|c|wasm|native-library|native-executable`，默认 `inspection`。`--cpu` 只允许
两个 Native consumer，缺省为 `baseline`；`native-executable` 必须存在合法 `main`，
Native consumer 必须启用 native-toolchain。

## B4 复诊：成立

LLVM 官方 `TargetTransformInfo` API 明确区分 reciprocal throughput、latency、code size
和 size-plus-latency cost kind；`InstructionCost` 也明确区分 valid/invalid 状态，并以
有符号整数承载成本。见 LLVM 的
[`TargetTransformInfo` API](https://www.llvm.org/doxygen/classllvm_1_1TargetTransformInfo.html)
和 [`InstructionCost` API](https://llvm.org/doxygen/classllvm_1_1InstructionCost.html)。
当前 bridge 只创建 `TargetMachine` 并在 LLVM lowering 后运行 PassBuilder，没有 profile
probe module/function，也没有 TTI query ABI。

修订固定：同 TargetMachine 的 synthetic module/function、CPU/feature attributes、
`TCK_RecipThroughput`、有限 operation/type/lane/alignment 查询域、逐类 TTI API、
legalization parts metadata、invalid/negative/overflow/zero 处理和 canonical SHA-256
序列化。TTI 返回成本已包含 target lowering 时不再重复乘 legalization parts。

## M1 复诊：成立

仓库只有 `CKCOBJ01` Native object/run cache entry、cache key schema 2、manifest schema 2
及可打印 KIR；没有持久化、可反序列化的 KIR artifact cache。现有 `kir_contract_version`
只是 object cache identity 的一个字段。

修订明确 0.12 不新增 KIR artifact cache。KIR print/schema identity 升为 `kir-v2`；已有
Native object/run cache 升为 entry `CKCOBJ02`、key schema 3、manifest schema 3，并把
完整 target-profile digest、vector cost/proof schema 和预算 schema 纳入 key/manifest。
旧 entry 在解码或 key identity 阶段必然拒绝。

## 复诊后的修订准则

上述修订保持原性能、安全和验收门槛，不删除任何目标，也不以实现困难为由降级。
其作用仅是让 transformation transaction、候选选择、CLI identity、target cost 和
cache compatibility 形成唯一可实现、可验证的闭环。修订双语设计后必须重新执行一轮
完整对抗性审查。
