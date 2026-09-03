# 阶段 17 验收：Floyd Assets 与 Runner

## 本地必须通过

- [ ] `cargo test --locked --test performance predicated_update_assets_ -- --nocapture`
- [ ] `cargo test --locked --test performance predicated_update_generator_ -- --nocapture`
- [ ] `cargo test --locked --test performance predicated_update_runner_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test performance predicated_update_native_runner_ -- --nocapture`
- [ ] `python3 -B -m unittest discover -s tests/performance -p 'tune_gate_test.py'`

## 结构与正确性

- [ ] 五个新 asset 的 exact bytes、path、N/seed、manifest expected digest 与
  Contract 1 相同。
- [ ] SplitMix64/generator golden cells 与三份完整 expected-result digest 匹配。
- [ ] 四协议 exact argv、环境、回执、错误行为和 release exclusion 完整。
- [ ] slice len 为 checked `n*n`；每个 timed call 使用 fresh matrix，timer 内只含
  native Floyd 调用。
- [ ] profile run 只 flush 一次并产生一个可解析 completed shard。
- [ ] malformed/overflow/over-cap/nonmatching/nonfinite/loader/symbol/flush failure
  全部非零退出且没有成功 receipt。

## 完成证据

asset SHA、golden digest、协议正负矩阵与 timer-call count 写入
`target/acceptance/v0.14/stage-17/`。
