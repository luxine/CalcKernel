# 阶段 07 验收：CKTJNL01 crash consistency

## 必须通过

- [ ] `cargo test --test tune journal_ -- --nocapture`
- [ ] `cargo test --test tune publication_ -- --nocapture`
- [ ] `cargo test --test tune recovery_ -- --nocapture`
- [ ] `cargo test --all-features --locked`

## 结构断言

- [ ] destination canonicalization、alias/short-name、complete overlap closure 与 lock order 使用同一 full id。
- [ ] persistent lock 初始化和 journal 更新只使用 flush 后 atomic no-replace/replace final names。
- [ ] Windows private initializer 的创建 handle 显式包含 `WRITE_DAC`，同一 handle 的 owner-only protected DACL 设置与验证在 required Windows x64/ARM64 job 成功；ACL 设置失败必须删除未保护文件并 fail closed。
- [ ] Windows directory flush handle 显式请求 `GENERIC_WRITE`，实际 directory open/flush 失败仍为 hard error；不得忽略失败或以 write-through rename 代替规范要求的 barrier。
- [ ] exact journal bytes、generation、direction、phase、role layout 与 OutputSetMaterial 均独立重算。
- [ ] 每个 publication/barrier crash point 都恢复为完整 old 或完整 new set，primary-last 不被破坏。
- [ ] impossible digest/metadata/orphan 组合保存证据并 fail-closed；rollback/roll-forward 重入幂等。

## 完成证据

记录被测 SHA、平台原子/flush capability、故障点矩阵、old/new digests、恢复方向和测试计数到 `target/acceptance/v0.14/stage-07/`。
