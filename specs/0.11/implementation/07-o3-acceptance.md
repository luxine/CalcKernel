# 阶段 07 验收：O3

当前复审状态：I18 发现命名 `induction-simplify` pass 没有实际 transform，本阶段对应
验收重新打开。不得仅以 pass-order 测试替代改写正例/反例；见
`../review/implementation-blockers-01.md`。

## 必须通过

1. `cargo test --locked --test optimizer kir_o3_ -- --nocapture`
2. `cargo test --locked --test optimizer loop_ -- --nocapture`
3. `cargo test --locked --test optimizer generated_loop_ -- --nocapture`
4. `cargo test --locked --test optimizer guard_ -- --nocapture`
5. `cargo test --locked --test ir proof_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- canonical provable slice loops 在 O2/O3 KIR hot loop 内无冗余 bounds guard。
- 近邻反例保留 guard 并给出稳定 conservative reason。
- LICM/induction mutation 不得改变 checked first-error、print order 或 strict f64。
- fixed-seed generated loops 在 O0–O3 observable behavior 一致。
- KIR 中没有 SIMD、unroll、versioning 或 specialization operation。

## 完成证据

执行时追加 SHA、loop fixture seed、guard count 和 verifier mutation count。

## 执行记录（2026-08-29）

- 实现提交：`4fd2ca09c30e022e627e547ae9b1c23c5f1ed4bc`
- O3 pipeline：17 项固定 pass record 全部 `verified=true`。
- loop fixtures：嵌套循环、multiple latch、break/continue 识别为 2 层 natural-loop tree；
  u32 strict-bound `+1` induction 由 entry/backedge SSA 重建。
- LICM：只 hoist 无 Memory SSA、无 ordered effect、无 checked failure 的 modular 纯值；
  checked arithmetic、runtime print、load/store/call 与 strict-f64 均不进入候选集合。
- canonical slice：同一 fixture 的 bounds guard 数为 O1=`1`、O2=`0`、O3=`0`；缺少
  `len <= items.len` 契约的近邻在 O3 仍为 `1`，reason 保持 conservative。
- loop proof：checker 同时复诊 loop header taken edge、phi entry/backedge、step、strict bound
  与 contract affine predicate；证明未开启 loop reasoning 的 O1 不会意外删除。
- generated fixture seed：`0xC0DE`，3 个 break/continue 小循环 × O0–O3 共 12 个 artifact
  全部通过 verifier，未新增 SIMD/unroll/versioning/specialization KIR operation。
- 本文件“必须通过”第 1–8 项全部通过；另执行 `cargo test --locked`，默认特性全仓
  326 个测试通过。
