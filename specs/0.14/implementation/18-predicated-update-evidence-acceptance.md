# 阶段 18 验收：Contract 1 证据链

## 本地必须通过

- [ ] `cargo test --locked --test performance predicated_update_contract_ -- --nocapture`
- [ ] `python3 -B -m unittest discover -s tests/performance -p 'predicated_update_gate_test.py'`
- [ ] `cargo test --locked --test contracts ci_v014_predicated_update_ -- --nocapture`
- [ ] checker fixture 的合法 report 通过，全部 mutation report 非零退出。

## 支持 tier 的本地或 stable host 必须通过

- [ ] `cargo bench --features native-toolchain --bench tune_perf -- --task collect-predicated-update --out target/ckc-perf/v0.14-predicated-update-results.json`
- [ ] `python3 scripts/check-v014-predicated-update.py target/ckc-perf/v0.14-predicated-update-results.json --schema-nine target/ckc-perf/v0.14-results.json`

## 闭合断言

- [ ] exact top-level/nested keys、typed digests、recipe、candidate/compiler/
  toolchain/hardware 与 schema 9 外键全部匹配。
- [ ] 七命令、profile directory/sole shard、四 cache namespace、tuned-only locks、
  artifacts/decision/attestation 和完整 evidence inventory 闭合。
- [ ] exactly-one target choice、minimum<=128、true guards、executed vector chunk
  均由 source-aware重建证明；复合 plan/unreachable rewrite mutation 被拒绝。
- [ ] 三个 split correctness digest、order、3/20/3 raw receipts、min/upper median、
  16-of-20、102/100 与 95/100 用整数重算。
- [ ] collector 源码不含 acceptance threshold 判定；checker 不计时、不选择
  evidence、不接收 post-result exclusion。

## 完成证据

contract/mutation 结果写入 `target/acceptance/v0.14/stage-18/`；真实 report 与
evidence tree 保存在 `target/ckc-perf/` 或 CI artifact，不提交。
