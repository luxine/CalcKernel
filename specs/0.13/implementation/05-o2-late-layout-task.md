# 阶段 05 任务：O2 CK late machine layout 与 bridge ABI 4

## 目标

在 LLVM 22.1.8 Native bridge 中建立可审计的 O2 权限边界：完整 ordinary IR/structural machine
pipeline 对 profile 完全盲，冻结 machine snapshot 后仅由 `CkLateProfileLayout` 重排 block/function/
section，并只允许 target closed allowlist 中的 terminator/fallthrough repair、branch relaxation、fixup、
padding 与 emission。任何越界 delta 拒绝 layout 并保留 ordinary order。

## 仓库落点

- 修改 `native/bridge/{ckc_llvm.h,ckc_llvm.cpp}`，以公开 `TargetPassConfig` 边界构建 target
  pipeline，新增 snapshot/layout/verify API 与 per-target repair allowlist；bridge ABI 升至 4。
- 修改 `src/backend/llvm/{ffi.rs,passes.rs,module.rs,object.rs,verify.rs,fact_audit.rs}`，新增
  `CkLateProfileLayoutPlan/Report`、checked KIR-to-LLVM/Machine mapping 与 fallback explanation。
- 修改 cache/build manifest/contract 中的 bridge identity，但暂不升级最终 bundle cache schema。
- 新建 `tests/native/pgo_layout.rs`，扩展 LLVM IR/object/disassembly/fact audit、bridge header/ABI 与
  six-target fixture tests。

## TDD 顺序

1. 写 ABI/schema RED：Rust/C header 完全匹配 ABI 4，传入 target/profile mapping/layout plan，返回
   pre/post structural digest、accepted delta 与 rejection reason；malformed buffer/enum/length fail-closed。
2. 写 O2 boundary RED：profile-on/off 在 `CkLateProfileLayout` 前的 IR/Machine snapshot byte-identical；
   LLVM 无 branch weight/entry count/hot-cold/profile summary 或 profile-derived attribute。
3. 写 legal layout RED：只允许 block/function/section permutation 和为 fallthrough 必需的 terminator
   repair；unmapped/unknown block 保持 ordinary relative order，stable tie-break。
4. 写 mutation RED：inline/vector/CFG/tail duplication、outline/split/merge/delete/clone、非 terminator
   change、call-target change、instruction reschedule 或 metadata leak 均由独立 pre/post verifier 拒绝。
5. 写 target repair RED：x86-64/AArch64 × ELF/Mach-O/COFF 的 allowlist 闭合；CFI/unwind/LOH/
   security/bundle 需要未列 repair 时 normal fallback；AArch64 accepted reorder 后重跑 required relaxation。
6. 写 object/MIR/disassembly RED：accepted on/off delta 只含顺序、required repair、relaxation、alignment；
   strict-f64、checked failure、public symbols、feature set、unwind/CFI correctness 不变。
7. 实现 target pipeline interception、snapshot serializer、layout application 与 verifier；将 fallback
   接入 stable explanation，bridge error 时 withholding object，normal unsupported repair 时 ordinary emit。

## 实现边界

- 结构边界而非 pass 名称定义权限；不得为了某 target 成功而隐式扩大 allowlist。
- O2 不执行 profile-driven inlining/specialization/loop/vector/multiversion，后者只在 O3。
- branch relaxation/padding 是接受 reorder 后的 target emission 必需步骤，不是第二轮优化机会。

## RED/GREEN 证据

记录 ABI mismatch RED、profile-on/off pre-layout snapshot digest、每 target legal/illegal delta、fallback
reason 与 object disassembly 到 `target/acceptance/v0.13/stage-05/`。
