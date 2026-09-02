# 阶段 08 验收：cache、CLI 与端到端 tune/replay

## 必须通过

- [ ] `cargo test --test tune cache_ -- --nocapture`
- [ ] `cargo test --test cli tune_ -- --nocapture`
- [ ] `cargo test --test tune session_ -- --nocapture`
- [ ] `cargo test --test native cache_ -- --nocapture`
- [ ] `cargo test --all-features --locked`
- [ ] `bash scripts/audit-native-artifact.sh <stage-executable> executable`
- [ ] `bash scripts/test-sanitized-ownership.sh`

## 结构断言

- [ ] CLI precondition/option/output matrix 闭合，所有早期失败无 output/cache/harness side effect。
- [ ] compile/measurement/decision cache 身份分离，salt/private permissions/checksum/atomic/LRU/4 GiB 完整。
- [ ] cold determinism 与 warm zero-work exact reuse 通过 locked inventory/event-log 证明。
- [ ] tune-use 调用 source-aware checker，任何 stale identity/frontier/plan/artifact mismatch 无 silent fallback。
- [ ] ordinary commands 对 tune-v1 零访问、零 harness、零 optimizer behavior change。

## 完成证据

保存被测 SHA、CLI matrix、cache inventories/events、cold/warm decision/output digests、artifact audit 和测试计数到 `target/acceptance/v0.14/stage-08/`。
