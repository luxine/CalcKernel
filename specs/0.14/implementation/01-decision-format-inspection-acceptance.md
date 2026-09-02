# 阶段 01 验收：CKTUNE01 与 inspection schema 1

## 必须通过

- [ ] `cargo test --test tune decision_ -- --nocapture`
- [ ] `cargo test --test tune inspection_ -- --nocapture`
- [ ] `cargo test --locked`
- [ ] `cargo fmt --all -- --check`

## 结构断言

- [ ] 唯一 schema 常量位于 `src/tune/schema.rs`；magic、enum、bounds 和 domain 字符串没有 CLI/backend 副本。
- [ ] 五个 canonical fixture 的 SHA-256 固定，decode/encode/re-encode、mutation、cross-endian 共用同一组 bytes。
- [ ] decoder 在 allocation 前完成 checked bound，拒绝 unknown/duplicate/out-of-order/trailing/non-NFC。
- [ ] self-contained checker 覆盖 Contract、Environment、Expansion、stream、Selection、Replay 的所有可派生等式。
- [ ] JSON/text 均从同一 validated tree 生成，输出 exact ordering 且不泄露 path、secret、timestamp 或 PID。

## 完成证据

将被测 SHA、RED 失败、fixture digest、命令及测试计数写入 `target/acceptance/v0.14/stage-01/`。

