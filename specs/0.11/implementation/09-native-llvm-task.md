# 阶段 09 任务：Native LLVM lowering 与事实审计

## 目标

让 Native object/library/executable/ORC 路径消费 verified KIR；实现 CK-owned strengthening
白名单、FactId/ProofId audit map 和严格位于 LLVM optimization 前的 fact audit。

## 仓库落点

- `src/backend/llvm/lower.rs`/`layout.rs`/`entry.rs`/`abi.rs` 输入迁移为 KIR；Native C ABI
  classifier 与 thunk shape 保持不变。
- 新建 `src/backend/llvm/fact_audit.rs`；在 typestate 中加入
  `VerifiedNativeModule -> AuditedNativeModule -> OptimizedNativeModule`。
- 扩展 `native/bridge/ckc_llvm.{h,cpp}` 与 `src/backend/llvm/{ffi.rs,builder.rs}`，只提供白
  名单 metadata/attribute/flag API 与枚举审计 API；private bridge ABI 从 1 升到 2。
- `tests/native/fact_audit.rs`、迁移 native LLVM/ABI/object tests。

## TDD 顺序

1. 写 KIR O0 Native structural differential red tests；逐类实现 scalar/control/slice/
   checked guard/runtime call lowering。
2. 写 typestate red/compile-shape tests：未 audit module 不能进入 LLVM optimize/object emit。
3. 为 range、alignment、nuw/nsw、memory effects、access-scoped alias metadata 分别写正例；
   lowering 每次设置 strengthening 时必须同时注册 FactId/ProofId。
4. 写 intentionally injected untracked attribute/flag mutation；audit 必须失败。基线 ABI
   attrs（zext/sext/sret/byval 等）使用独立固定白名单，不伪装成优化事实。
5. 写 pairwise noalias third-root/capture/return 反例；仅完整参数承诺才能生成 parameter
   `noalias`，否则只允许 scoped metadata。
6. 写 audit timing test：CK audit 在 `.verify()` 后、`.optimize()` 前；LLVM O3 后自行推导
   的属性不回送 CK audit。
7. 跑 O0–O3 Native differential、ABI、object/library/executable/ORC tests。

## 实现判定

- LLVM lowering 不执行 target-neutral analysis，也不从模式补 guard。
- 每个 CK strengthening 都有可枚举 ownership、source Fact/Proof 和 admissibility kind。
- audit failure 发生在 output transaction 前，禁止 artifact。
- LLVM 后续 canonicalization/isel/regalloc/scheduling 仍由 LLVM 负责。

## 明确不做

不开放 public LLVM/JIT API，不实现 cross-compilation，不实现 0.12 target specialization。
