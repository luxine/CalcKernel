# CK 0.12 设计第五轮对抗性审查（通过）

日期：2026-09-01
审查对象：第四轮修订后的双语 CK 0.12 设计
结论：**通过；无阻断项，无未关闭重大隐患。**

## 审查范围

本轮从零重新检查，而不是只看最后一个补丁，覆盖：

- scalar MIR 到 KIR v2、Vector/Mask value、Memory SSA 与 public ABI 边界；
- target profile 的有限全集、TTI query、legalization、成本归一化与 digest；
- canonical loop、dependence、version predicate、checked first-error 与 strict f64；
- proposer/checker 独立性、verified program transaction、audit/budget 单调状态；
- specialization、Loop SIMD、full/partial unroll、SLP 的候选覆盖和稳定总顺序；
- Native/C/Wasm/inspection consumer、CLI 参数与 backend lowering；
- KIR schema、Native object/run cache、bridge/public/runtime ABI；
- differential、mutation、disassembly、feature containment、performance、size、compile-time 与
  exact-SHA 六主机/十作业门禁。

## 阻断项关闭矩阵

| 项 | 最终关闭条件 |
| --- | --- |
| B1 | 专用化只复制并原子交换 `KirVerifiedProgramState`，不提前运行正式 loop/vector 流水线。 |
| B2 | 同一 canonical scalar pre-state 上比较 Loop SIMD 与 unroll/SLP alternative，每 loop 至多提交一个 winner。 |
| B3 | `emit-kir --consumer` 与现有五个 `KirConsumer` 精确映射，Native CPU/main 规则明确。 |
| B4 | Synthetic TTI probe、`TCK_RecipThroughput`、逐类 query、cost/legalization 归一化和 canonical digest 固定。 |
| B5 | Scalar full/partial unroll 与 unroll+SLP 固定 10% 加两个 cost unit，Loop SIMD 保持 20%。 |
| B6 | Probe lane domain 固定为 `{2,4,8,16}` 且最多 512 bit，全集显式 Legal/Unavailable。 |
| B7 | `KirOptimizationAuditState` 不回滚；拒绝、复用和未获胜 candidate 永久计费并保留稳定原因。 |
| B8 | Specialization、loop frontier、residual SLP 各有稳定 stage key；scalar-only 与 SLP unroll variant 均被覆盖。 |
| M1 | 明确无持久化 KIR cache；KIR schema v2 与现有 object/run cache `CKCOBJ02`/schema 3 分开升级。 |

## 逻辑闭环复核

1. **安全闭环**：Vector memory footprint 不越界；checked candidate 必须先证明所有 lane 不会
   failure；runtime address predicate overflow 只走原始 scalar fallback；strict f64 禁止 contraction/
   reassociation；错误 certificate withholding artifact。
2. **事务闭环**：未接受的 module/fact/proof mutation 全部回滚，尝试预算和 rejection audit
   全部保留；接受后运行结构与 evidence verifier，不存在半提交。
3. **确定性闭环**：profile universe/digest、candidate order、integer threshold、budget、tie-break
   和解释 reason 均不依赖 wall clock、hash iteration 或 machine load。
4. **后端闭环**：只有 Native profile 允许 Vector KIR；C/Wasm 保持 verified scalar KIR；Native
   bridge/profile/lowering 使用同一 TargetMachine identity；public Native ABI 1 和 Runtime ABI 2
   不变。
5. **验收闭环**：结构测试避免“LLVM 自己向量化”的假阳性；object disassembly 证明真实 SIMD；
   scalar replay、手写 C/Rust SIMD、domain-fact、size 和 compile-time 门槛互相独立且均为 fail-
   closed；最终以 exact candidate SHA 的 CI 结果为准。

## 非阻断风险与计划要求

以下是实现风险，不是设计缺口，必须在阶段计划中显式控制：

- KIR type migration 横跨 builder、printer、validator、optimizer 和三个 backend，应先建立
  schema/validator 红测，再逐层迁移，避免一次性无证据重写。
- Native TTI 与 vector builder 扩充 private C ABI，应先做 owner/error/mutation test，并在
  ASan/UBSan job 保持覆盖。
- 0.11 replay、C/Rust SIMD oracle 与 schema 7 规模大，应在 optimizer feature 完成前建立
  harness contract tests，最后才运行稳定 worker 性能门禁。
- 当前本机没有设置 `CKC_LLVM_PREFIX`，非 Native 基线已通过；Native 完整验收需 bootstrap
  pinned LLVM 或使用 CI，远程长任务应定期查询而非阻塞等待。

## 最终判定

双语设计现在可以进入实施计划拆分。后续若实现揭示真实设计错误，可以按用户授权修订，
但不得为通过测试、性能或 CI 而降低这里冻结的语义、预算、性能或 exact-SHA 门槛。
