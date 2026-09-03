# 阶段 13 任务：Host Artifact 路径与 LLVM Void Call

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

关闭两个与优化无关但会阻断六 host 的 Native 根因：测试硬编码 Darwin 动态库
文件名，以及 LLVM 22 对 void call 赋 SSA 名称时触发断言。修复必须覆盖真实
executable/dynamic build，不改变 Bridge ABI 4。

## 仓库落点与接口

- 修改 `tests/cli/commands.rs` 的
  `multiversion_build_should_commit_the_verified_stage09_artifact_bundle`，用
  `NativeArtifactPaths::new(NativePlatform::host(), NativeArtifactKind::Dynamic,
  &base)` 计算 primary/header/import-library，并把 logical base 传给 CLI。
- 修改 `native/bridge/ckc_llvm.cpp::ckc_llvm_builder_call`：先无名
  `CreateCall`，仅在 return type 非 void 且 name 非空时 `setName`。返回的
  `CallInst` 仍通过原 handle 返回，C header、Rust FFI 和 Bridge ABI 不变。
- 扩展 `tests/native/bridge.rs` 与 `tests/native/llvm_ir.rs`，覆盖命名非 void
  call、命名 void call、空名 void call、profile flush void call 和模块 verify。
- 扩展 `tests/contracts/ci.rs`，要求六 host 运行非零 selector 的 executable、
  dynamic、profile-generation 和 void-call 回归。

核心修复形状固定为：

```cpp
auto *call = builder->value->CreateCall(
    callee->getFunctionType(), callee, values);
if (!callee->getReturnType()->isVoidTy() && name.length != 0) {
  call->setName(borrowed_string(name));
}
*out = bridge_value(call);
```

## TDD 顺序

1. 将 CLI 测试改写为 host-resolved 期望并先运行 Linux 当前用例，确认旧硬编码
   `libadd.dylib` 不能满足 Linux primary 路径。
2. 添加 RED `named_void_call_should_verify_without_an_ssa_result_name`，直接构造
   void callee，以非空名称调用 bridge，要求 LLVM IR 含 `call void` 且模块
   verify 成功；在旧 bridge 上确认 LLVM 22 失败。
3. 实现无名 CreateCall + conditional setName；运行 bridge/IR selector，确认
   非 void 结果仍保留名称，void call 没有非法赋名。
4. 运行真实 profile-generation fixture，覆盖 `__ck_profile_initialize`、flush
   与 runtime void calls；不得用字符串 fixture 代替 bridge 执行。
5. 在 CI contract RED 中要求六 host 明确执行
   `native_artifact_host_paths_`、`llvm_void_call_`、`profile_generation_`，并在
   workflow 加入对应步骤；不改变十 job 数量。
6. 运行阶段验收并记录 `target/acceptance/v0.14/stage-13/`。

## 阶段命令

```sh
cargo test --all-features --locked --test cli multiversion_build_should_commit_the_verified_stage09_artifact_bundle -- --nocapture
cargo test --all-features --locked --test native named_void_call_ -- --nocapture
cargo test --all-features --locked --test native profile_generation_ -- --nocapture
cargo test --locked --test contracts ci_v014_native_fulfillment_ -- --nocapture
```

## 边界

- 不 suppress LLVM assertions、不丢弃 CallInst、不为 void 制造 dummy value。
- 不按 `cfg!` 在测试中手拼 suffix；路径权威始终是 `NativeArtifactPaths`。
- 不改变 public header、FFI 参数、返回 status 或 bridge ownership。
