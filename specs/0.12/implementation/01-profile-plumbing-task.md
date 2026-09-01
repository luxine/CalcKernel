# 阶段 01 任务：KIR v2 profile 与 consumer plumbing

## 目标

在不生成 Vector KIR 的前提下建立 0.12 的确定性 target-profile 外壳，把 exact profile
identity 贯穿 KIR builder、pass manager、printer 和 `emit-kir`。O0–O3 的既有 scalar 行为
与 pass 顺序必须保持。

## 仓库落点

- 新增 `src/ir/kir/profile.rs`，并由 `src/ir/kir/mod.rs` 导出。
- 修改 `src/ir/kir/{model,builder,print,validate}.rs`。
- 修改 `src/optimizer/kir_pipeline.rs` 及所有调用点。
- 修改 `src/cli/{args,commands}.rs`、CLI/help 测试。
- 测试落在 `tests/ir/kir.rs`、`tests/optimizer/preservation.rs`、
  `tests/cli/{commands,kir_inspection}.rs`。

## TDD 顺序

1. 先写 `profile_` RED：schema 1、Inspection/C/Wasm 三种 profile 的 identity/layout/
   vector-disabled 状态、固定 tagged encoding、SHA-256 digest、重复构造 byte-identical；
   修改任一 scalar cost/layout/consumer 必须改变 digest。
2. 写 malformed profile RED：缺 key、重复 key、非法零/负语义、未知 layout 却启用 layout-
   sensitive operation，validator 必须 fail-closed。
3. 写 KIR binding RED：module 打印 `kir-v2 profile-schema=1 profile=<digest>`；传入不同 profile
   或 stale digest 时 structural/evidence verifier 拒绝，且无 artifact。
4. 写 pipeline preservation RED：O0–O3 每次命名 pass 仍运行 verifier；0.11 fixtures 的 scalar
   KIR 除 header/profile 与类型拼写的预期 schema 变化外，CFG/effect/guard 不漂移。
5. 写 CLI RED：`--consumer inspection|c|wasm|native-library|native-executable` 精确解析，默认
   inspection；`--cpu` 对前三种非法，Native 缺省 baseline；feature-disabled Native 返回稳定
   unavailable error；`native-executable` 无 main 返回与 build/run 同一规则。
6. 最小实现 profile、threading 与 parser；重构时所有 map 使用有序结构，digest 不使用 Debug
   输出、平台字节序或 hash iteration。

## 实现边界

- 本阶段 Native profile 只定义数据模型和 feature-disabled 错误，不查询 LLVM。
- Profile query universe 的五 lane type、`{2,4,8,16}`、512-bit cap 和 operation key enum 必须
  已经固定，但所有 non-Native vector entry 都为 Unavailable。
- KIR schema identity 升为 2；private LLVM bridge/cache/package version 暂不修改。
- 不增加 Vector instruction，不做 loop/specialization/unroll/SLP。

## RED/Green 证据

在已忽略的 `target/acceptance/v0.12/stage-01/` 记录每组首个 RED test、失败原因、对应最小
实现与最终 test count。不得只写“测试先失败过”，也不得回写动态证据改变被测 SHA。
