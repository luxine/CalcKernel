# 阶段 03 任务：LLVM TTI profile、Vector lowering 与 cache/bridge v3

## 目标

用同一个 `NativeTarget` 在 KIR 优化前构造可复现 `KirTargetProfile`，扩充 private bridge ABI
和 Native lowering，使经过验证的 Vector KIR 结构化 lowering 为 LLVM fixed vectors；同步
升级 Native object/run cache，仍不启用自动向量变换。

## 环境与落点

- 先按 README bootstrap/validate LLVM 22.1.8 release prefix，设置 `CKC_LLVM_PREFIX`。
- `native/bridge/ckc_llvm.{h,cpp}`、`src/backend/llvm/{ffi,target,builder,kir_lower,fact_audit}.rs`。
- `src/cli/cache/{key,entry,store}.rs` 与 Native compile flow。
- 测试：`tests/native/{bridge,llvm_ir,fact_audit,cache,ownership}.rs` 与 contracts。

## TDD 顺序

1. 写 bridge ABI 3/owner/error RED：synthetic module/function、triple/layout、CPU/排序 features、
   null/invalid handle、重复 dispose/move ownership 继续由 Rust 类型和 C ABI 防护。
2. 写 TTI canonical profile RED：`TCK_RecipThroughput`，固定 lane universe，每 key 恰一次，
   operation-specific cost、legalization parts/type；invalid/negative/overflow/non-whitelist-zero
   为 Unavailable。同 target 重建 bytes/digest 相同。
3. 写 baseline/native containment RED：x86-64 baseline 只承诺 mandatory SSE2，AArch64 baseline
   只承诺 generic ARMv8-A/Advanced SIMD；native 只能使用 resolved feature string。
4. 对每个 Vector KIR family 写 LLVM IR RED；验证 exact fixed-vector/mask type、strict f64 flags、
   alignment 不增强、Memory SSA 映射与 reduction intrinsic/sequence。
5. 写 audit mutation RED：未携 proof/profile 的 vector strengthening、stale cost、非法 alignment、
   vector op 超出 profile 必须在 LLVM optimization 前拒绝。
6. 写 cache RED：`CKCOBJ02`、key schema 3、manifest schema 3；profile digest/cost/proof/budget
   schema 任一变化改变 key；旧 `CKCOBJ01`/schema 2 entry 拒绝。
7. 跑 ASan/UBSan ownership 与 actual object smoke；不依赖 LLVM 后置 vectorizer 代替结构 lowering。

## 实现判定

- TTI cost 已含 lowering，不重复乘 legalization parts；scalarized/unsupported form 不 Legal。
- Native command 必须先建 target/profile，再建/优化 KIR，再以同一 target lowering/PassBuilder。
- Bridge 只暴露 POD/owned-byte ABI，不跨 FFI 泄露 LLVM C++ object 或异常。
- Bridge ABI=3；public Native ABI=1、Runtime ABI=2；package version仍暂为 0.11.0。

## 明确不做

不启用 loop/SLP vectorizer，不加入 source intrinsics，不修改 C/Wasm backend，不运行最终性能门禁。
