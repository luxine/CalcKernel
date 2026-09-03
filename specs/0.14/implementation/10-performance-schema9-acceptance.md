# 阶段 10 验收：schema 9 本地契约与真实性基础

> **当前定位：已落地的累计基础。** 本阶段的 schema 9 通过不能替代独立的
> Predicated-Update Contract 1；最终候选仍须通过阶段 17–19。

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
- [ ] 稳态 `elapsedNs` 只覆盖 native runner 内的 kernel 迭代循环；启动、加载、分配、摘要和 Python/FFI 开销均在计时外。
- [ ] evidence root 中每个真实文件恰有一致的 evidence `FileIdentity`；无未引用 compile/profile/cache/lock scratch。
- [ ] 每条 `Command.argv` 原样传给子进程；需要安装布局的 oracle 只以 checker 验证过的等字节原映像作为 executable，不改写 argv。
- [ ] C/Rust oracle 在严格空环境中分别通过显式 `--ld-path` 与 `-C linker/-C link-arg` 使用固定
  Clang 和 `/usr/bin/ld`；二者身份进入 toolchain/Command.inputs 且由 checker 现场复核。
- [ ] x86-64-v4 feature gate 精确包含 AVX-512CD，缺一项即 fail-closed。
- [ ] 七个 v0.13 `.ckprof` 均以 CKPROF01 扁平 `compilerSource`、精确 target/mode 和完整 observation 内容独立 inspect。

## 完成证据

本地契约证据写入 `target/acceptance/v0.14/stage-10/`；真实性能 report/evidence 只保留在 `target/ckc-perf/` 或 CI artifact，不提交。
