# 阶段 11 验收：十作业 exact-SHA CI

## 本地必须通过

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --all-targets --all-features -- -D warnings`
- [ ] `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features`
- [ ] `cargo test --locked`
- [ ] `cargo test --all-features --locked`
- [ ] `python3 -B tests/performance/tune_gate_test.py`
- [ ] `python3 -B scripts/audit-performance-oracles.py --tune`
- [ ] `bash scripts/test-sanitized-ownership.sh`
- [ ] `git diff --check` 且 tracked/untracked 工作区为空。

## 远程必须通过

- [ ] quality 对最终 SHA 成功。
- [ ] native-integration 对最终 SHA 成功。
- [ ] 六个 Native host job 对最终 SHA 成功且没有 required skip。
- [ ] Linux x86-64-v4 与 AArch64 SVE2 performance job 对最终 SHA 完整通过 schema7/8/9。
- [ ] workflow run head SHA、十 job checkout SHA、schema9 `candidateSha`、上传 evidence 与本地最终提交相等。

## 完成证据

本地记录放 `target/acceptance/v0.14/stage-11/`；远程 run id、job conclusions 与 artifact digests 放 CI artifact/外部审查记录，不回写 candidate SHA。

