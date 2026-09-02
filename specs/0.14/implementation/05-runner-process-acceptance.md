# 阶段 05 验收：runner、process control 与 calibration

## 必须通过

- [ ] `cargo test --test tune runner_ -- --nocapture`
- [ ] `cargo test --test tune calibration_ -- --nocapture`
- [ ] `cargo test --test native run_ -- --nocapture`
- [ ] `cargo test --all-features --locked`

## 结构断言

- [ ] 无 shell/PATH lookup；空环境、protocol env、cwd 和 input-map 均精确且 bounded。
- [ ] Unix group 与 Windows Job 在用户代码前建立，timeout/overflow termination、reap、empty 顺序完整。
- [ ] Windows argv 使用 frozen UCRT inverse 与 `lpApplicationName`，golden probe 精确恢复全部边界参数。
- [ ] calibration/confirmation/overshoot/iterations 与 manifest expected digest 完整记录且 checked。
- [ ] complete candidate timeout 与 crash/protocol/correctness/budget admission 有不同 fail-closed typestate。

## 完成证据

写入被测 SHA、timer kind/resolution、平台 containment probe、timeout coordinates、calibration records 和测试计数到 `target/acceptance/v0.14/stage-05/`。

