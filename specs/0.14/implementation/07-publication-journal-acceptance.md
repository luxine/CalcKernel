# 阶段 07 验收：CKTJNL01 crash consistency

## 必须通过

- [ ] `cargo test --test tune journal_ -- --nocapture`
- [ ] `cargo test --test tune publication_ -- --nocapture`
- [ ] `cargo test --test tune recovery_ -- --nocapture`
- [ ] `cargo test --all-features --locked`

## 结构断言

- [ ] destination canonicalization、alias/short-name、complete overlap closure 与 lock order 使用同一 full id。
- [ ] persistent lock 初始化和 journal 更新只使用 flush 后 atomic no-replace/replace final names。
- [ ] exact journal bytes、generation、direction、phase、role layout 与 OutputSetMaterial 均独立重算。
- [ ] 每个 publication/barrier crash point 都恢复为完整 old 或完整 new set，primary-last 不被破坏。
- [ ] impossible digest/metadata/orphan 组合保存证据并 fail-closed；rollback/roll-forward 重入幂等。

## 完成证据

记录被测 SHA、平台原子/flush capability、故障点矩阵、old/new digests、恢复方向和测试计数到 `target/acceptance/v0.14/stage-07/`。

