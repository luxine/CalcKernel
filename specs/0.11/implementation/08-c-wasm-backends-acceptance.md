# 阶段 08 验收：C 与 WebAssembly KIR 后端

## 必须通过

1. `cargo test --locked --test backend kir_c_ -- --nocapture`
2. `cargo test --locked --test backend kir_wasm_ -- --nocapture`
3. `cargo test --locked --test backend -- --nocapture`
4. `cargo test --locked --test cli commands::cli_should_check_and_emit_portable_outputs -- --exact --nocapture`
5. `cargo test --locked --test performance bench::benchmark_harness_should_cover_compiler_stages_and_backends -- --exact --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- backend 模块不调用 MIR optimizer/range/alias helper，不根据 mode 私自创建 guard。
- C 与 WASM 在 O0–O3 的 supported-mode differential 全绿。
- pairwise noalias third-root case 不产生 C `restrict`。
- canonical checked C loop 的 backend hot loop 与 KIR 一样无冗余 guard。
- 0.10 C/WASM ABI snapshots 和不可达 runtime reachability 行为不漂移。

## 完成证据

执行时追加 SHA、编译器 identities、differential matrix 与 structural guard count。

## 执行记录（2026-08-29）

- 实现提交：`1035c70654960b3ccab9b649091b30d2653a7aa9`。
- KIR C：unchecked 与 checked/checked 两组 supported-mode matrix 均覆盖 O0、O1、O2、
  O3；每格分别运行 KIR-new 与固定 0.10 MIR-old 输出，scalar/control/void/slice/struct
  observable results 全部一致。额外覆盖 checked 首错、内部调用、扁平 slice 参数、
  `slice<Struct>` 前置声明及用户名与生成名冲突。
- KIR WebAssembly：unchecked matrix 覆盖 O0、O1、O2、O3；每格分别实例化 KIR-new 与
  MIR-old WASM，scalar/control/void/slice/struct 的返回值和线性内存结果全部一致。WAT 与
  binary 均通过同一 KIR-to-layout adapter；checked KIR 在 backend boundary 稳定拒绝。
- facts：完整两根 pointer/slice noalias 关系生成 2 个参数上的 portable
  `CKC_RESTRICT`，verified alignment 生成 conditional `CKC_ASSUME_ALIGNED`；只有
  `noalias(a, b)` 的三根反例生成 restrict 参数数为 `0`。
- canonical checked slice loop 的 C failure branch 计数：O1 bounds=`1`、O2 bounds=`0`、
  O3 bounds=`0`；已消除 guard 的 condition/overflow helper 不在 backend 重新生成。
- 回归计数：KIR C `7/7`、KIR WASM `3/3`、完整 backend `52/52`；portable CLI 与
  benchmark-harness 精确用例各 `1/1`。
- 工具链：`rustc 1.90.0 (1159e78c4 2025-09-14)`，host
  `aarch64-apple-darwin`，Rust LLVM `20.1.8`；`cargo 1.90.0`；Apple Clang
  `17.0.0 (clang-1700.6.4.2)`；Node `v24.14.0`。
- 本文件“必须通过”第 1–8 项全部以 exit status `0` 通过。
