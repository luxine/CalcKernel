# 阶段 10 验收：schema 9 本地契约与真实性基础

## 本地必须通过

- [ ] `cargo test --test performance tune_ -- --nocapture`
- [ ] `python3 -B tests/performance/tune_gate_test.py`
- [ ] `python3 -B scripts/audit-performance-oracles.py --tune`
- [ ] `python3 -B scripts/measure-v014-performance.py --contract-only --out target/ckc-perf/v0.14-contract.json`
- [ ] `python3 -B scripts/check-native-performance.py --schema-only target/ckc-perf/v0.14-contract.json`
- [ ] `cargo test --all-features --locked`

## 稳定性能 host 必须通过

- [ ] `cargo build --release --features native-toolchain --locked`
- [ ] `cargo bench --features native-toolchain --bench tune_perf -- --task collect --out target/ckc-perf/v0.14-results.json`
- [ ] `python3 -B scripts/check-native-performance.py target/ckc-perf/v0.14-results.json`

## 结构断言

- [ ] 七 case/manifest、三 partition、CK/C/Rust digest、recipe 与所有 evidence file identity 完整。
- [ ] schema 9 exact keys/cardinality/order/statistics/foreign keys/thresholds 均由 mutation tests fail-closed。
- [ ] v0.13 historical schema8 与 v0.14 cumulative schema8 分离且分别由正确 commit/checker 验证。
- [ ] wait4 RSS、compile/size/archive/cache/session、cold/warm determinism 都有 retained raw receipt。
- [ ] collector 无 accept 逻辑，checker 无重建/重计时/selective-rerun 权限。

## 完成证据

本地契约证据写入 `target/acceptance/v0.14/stage-10/`；真实性能 report/evidence 只保留在 `target/ckc-perf/` 或 CI artifact，不提交。

