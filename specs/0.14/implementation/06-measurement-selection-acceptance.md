# 阶段 06 验收：measurement、selection 与完整 decision

## 必须通过

- [ ] `cargo test --test tune measurement_ -- --nocapture`
- [ ] `cargo test --test tune selection_ -- --nocapture`
- [ ] `cargo test --test tune session_ -- --nocapture`
- [ ] `cargo test --test tune decision_ -- --nocapture`
- [ ] `cargo test --all-features --locked`

## 结构断言

- [ ] smoke/search/validation 的 cases/channels/rows/calls/rotation 全部由 immutable state machine 生成。
- [ ] timeout stream-set 可由 exact coordinate 重算；partial stream 不存储且 later slot 显式 skip。
- [ ] median/stability/Q32/paired wins/rank 使用 checked integer，未知/溢出/不稳定 fail-closed。
- [ ] selection 四行表与 candidate terminal matrix 一致，certificate 只授权 exact tuned plan。
- [ ] 完整 decision 同时通过 encode/decode/self-contained/source-aware 检查后才可交给发布层。

## 完成证据

保存被测 SHA、canonical event log、timeout mutation、两轮 summaries、decision digest 和测试计数到 `target/acceptance/v0.14/stage-06/`。

