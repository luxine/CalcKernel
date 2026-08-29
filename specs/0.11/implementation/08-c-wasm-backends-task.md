# 阶段 08 任务：C 与 WebAssembly 后端迁移

## 目标

让 C source/header 与 WAT/WASM 正式消费 optimized verified KIR，并用差分测试证明与
0.10 observable semantics/ABI 一致。开发 shadow emitter 可用于比较，但本阶段结束后
CLI 切换前的 KIR backend API 必须完整。

## 仓库落点

- 将 `src/backend/c/{layout.rs,emit.rs,checked.rs}` 的输入改为 KIR；复用 names/options
  与 ABI 类型布局，不复用 MIR 私有 guard 推导。
- 将 `src/backend/wasm/{plan.rs,emit.rs,binary.rs}` 的输入改为 KIR；保持 unchecked-only
  consumer capability rejection。
- `src/backend/header.rs` 暂保留 ABI shape；unsafe contract comments 在阶段 10 接入。
- `tests/backend/kir_c.rs`、`tests/backend/kir_wasm.rs`、迁移现有 backend tests。

## TDD 顺序

1. 为每个 scalar/control/void/slice/struct fixture 写 MIR-old 与 KIR-new differential red
   test；先只实现 O0 KIR emit。
2. 写 C checked status/first-error red tests；删除 backend 自建 bounds/overflow condition，
   逐个从 KIR guard/effect lowering。
3. 写 C fact hints red tests：只在完整证明时生成 portable conditional alignment 与
   `restrict`；pairwise third-root 反例不得生成 restrict。
4. 写 WAT/WASM byte/validation/runtime red tests；使用 KIR 已消除检查和 proven alignment，
   不发明 alias metadata/checked ABI。
5. 对 O0–O3、每个支持的 safety mode 跑 C/WASM differential；核对 export roots、不可达
   print 与 source/header transaction。
6. 将 benchmark harness 的 C/WASM compile-stage measurement 切到 KIR API。

## 实现判定

- backend 只翻译 KIR，不执行 target-neutral range/alias 分析，不补语义 guard。
- C fallback 在 GCC/Clang/MSVC-style preprocessing 下保持标准可用；hint 缺失只损失性能。
- WAT text 与 WASM binary 来自同一 KIR layout plan。
- public C/WASM ABI shape 与 0.10 fixtures 一致。

## 明确不做

不支持 WASM checked ABI，不实现 Native LLVM，不接 sanitizer CLI。
