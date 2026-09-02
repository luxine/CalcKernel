# 阶段 03 验收：frontier、search 与 plan checker

## 必须通过

- [ ] `cargo test --test optimizer tuning_ -- --nocapture`
- [ ] `cargo test --test tune frontier_ -- --nocapture`
- [ ] `cargo test --test tune search_ -- --nocapture`
- [ ] `cargo test --locked`

## 结构断言

- [ ] site/unit/variant/plan ID、root anchor、payload 与 ordering 全部 canonical 且 bounded。
- [ ] expansion trace 从 0 连续，覆盖实际每次 attempt，非法/重复/growth/limit 不退款。
- [ ] rank 使用 whole-plan metrics；beam、compile selection 和 later finalist 共用唯一 diversity implementation。
- [ ] plan 独立 checker 不调用 proposer，并从同一 immutable verified pre-state 重算 legality 与 post-state。
- [ ] ordinary empty-plan O3 及 C/WASM/O0–O2 行为与 v0.13 基线一致。

## 完成证据

记录被测 SHA、最小 forged-plan RED、frontier/plan digest、ordinary byte comparison 和测试计数到 `target/acceptance/v0.14/stage-03/`。

