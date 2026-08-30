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

## I18 循环输入/不变量绑定的局部复验（2026-08-30，尚非阶段通过）

- 本节对应 `optimizer(stage-07): bind induction facts to all incoming SSA edges`，parent
  为 `cf9dc0af5dc9110b550a7ab080528ec60080edef`。具体反例见 I18 第六部分，规范与验收
  门槛未改动。
- 实际 red 之一：有 +2/+1 两条 latch 的循环错误报告 step=1。现检查所有入口和回边，
  并以真实 SSA 转发替换 slot 名推断。混合步长/中间重赋值的近邻保持保守，原嵌套和
  一致多 latch 正例、LICM、canonical slice 检查消除仍通过；loop filter 为 8/8。
- 实际 red 之二：真实回边 i+1，却用未参与回传的 i+0 证明 i 始终等于 0，独立 checker
  原来错误接受。现每条 backedge 必须绑定声明 transfer 的正确类型首结果；该错误证书
  拒绝，真实回传 i+0 的合法近邻接受。Debug/release proof filter 均为 6/6。
- 顺序全量执行 `cargo +1.90.0 test --locked` 与
  `cargo +1.90.0 test --locked --all-features`，411/532 项全部通过（Native 92、CLI 21）；
  all-feature Clippy、fmt、diff check 通过。环境为 Rust 1.90.0 / AArch64 macOS /
  LLVM+Clang 22.1.8，沿用阶段 05 的固定 overlay identity。
- 默认、全特性与 release proof 日志 SHA-256 分别为
  `2d6717080d1693324c23e9fdb81e2ddcc559c88d66f13940e8556a8053503bf0`、
  `d577558c42a2ffd2e8fc8171d396c1ceb7f22833f3af1fba5719271a156cc930`、
  `2c7b76eecf4ad4f60cf5bdbe2647e593173b8b269f02b685962f04aab9abd5ac`。
- 本节只验收上述输入绑定修复。实际 induction simplification、guard loop checker 的
  独立性、irreducible/budget 及完整阶段任务仍打开；也未重跑或代签 I14/I19/I20 性能门槛。
