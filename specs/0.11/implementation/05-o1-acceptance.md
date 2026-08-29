# 阶段 05 验收：O0/O1

## 必须通过

1. `cargo test --locked --test optimizer kir_o0_ -- --nocapture`
2. `cargo test --locked --test optimizer kir_o1_ -- --nocapture`
3. `cargo test --locked --test optimizer guard_ -- --nocapture`
4. `cargo test --locked --test ir proof_ -- --nocapture`
5. `cargo test --locked --test optimizer runtime_print_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- O0 KIR 与 builder 输出除验证记录外相同。
- O1 pass order 与规范逐项一致，每项后都有 verifier record。
- 正例 guard 带有效 ProofId 消失；每个近邻反例保留并有确定 reason。
- checked 首错、print 和 may-fail mutation 全部拒绝非法 reorder/delete。
- 任一 invalid certificate 使 compilation failure，且 output transaction 未提交。

## 完成证据

执行时追加 SHA、每类 eliminated/retained guard 数与 mutation 结果。

## 执行记录（2026-08-29）

- 实现提交：`18bcf353cabd7f11734fcc9bcb17763d8eef81ef`
- O0：2 个 validator-only 用例通过；合法输入 KIR 保持不变，非法输入无 artifact。
- O1：固定 `cfg-canonicalize -> sccp-range -> check-elimination ->
  dead-code-elimination -> cleanup` 次序通过，五项 record 均由 verifier 标记为已验证。
- guard：常量安全溢出与支配的契约 slice 边界各删除 1 个；未知标量相邻例保留 1 个，
  reason 固定为 `retained: scalar safety is unknown`。
- mutation：将 `GuardSafety.condition_instruction` 改为不存在的 ID 后，独立 checker 拒绝，
  output transaction 无 artifact。
- 有序效果：`print_i32` 在 O1 DCE 后仍存在，cleanup 后 effect order 为 `[0]`。
- 验收命令：本文件“必须通过”第 1–8 项全部通过；另执行 `cargo test --locked`，
  默认特性全仓 308 个测试通过。
- 说明：额外探测的 `cargo test --locked --all-targets` 会执行 benchmark binary；既有
  `ckc_perf` 的 `emit-llvm-o3` 明确要求 `native-toolchain` feature，因此该非阶段门禁命令
  未记为通过，原生 feature 验证留在阶段 09。
