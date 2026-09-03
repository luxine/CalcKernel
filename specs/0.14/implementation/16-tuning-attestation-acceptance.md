# 阶段 16 验收：Tuning 与 Attestation

## 本地必须通过

- [ ] `cargo test --locked --test optimizer predicated_tuning_ -- --nocapture`
- [ ] `cargo test --locked --test tune predicated_attestation_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test cli predicated_attestation_ -- --nocapture`
- [ ] `cargo test --locked --test tune decision_ -- --nocapture`

## 结构断言

- [ ] 不同合法 VF/UF/minimum 保留为 distinct LoopSimd variants，且 replay
  产生匹配 post-state。
- [ ] attestation 只接受 exactly one PlanChoice/UnitVariant/SiteAlternative 的
  target Floyd shape，minimum<=128。
- [ ] source-aware checker 重建 compare/load/store、guard、pre/post 与所有 id；
  文本不能自证。
- [ ] tuned/replay line byte-equal、field/order/canonical encoding 精确；普通路径
  不输出该 prefix。
- [ ] cold、warm-cache 与 replay 在验证失败时均不发布部分输出。
- [ ] CKTUNE01 golden bytes、schema 1、Decision bounds 和 existing replay tests
  无变化。

## 完成证据

variant 列表、negative matrix、tuned/replay stderr identity 与命令输出写入
`target/acceptance/v0.14/stage-16/`。
