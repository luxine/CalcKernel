# 阶段 19 验收：CI 与最终交付

## 本地质量

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --all-targets --all-features --locked -- -D warnings`
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked`
- [ ] `cargo test --locked`
- [ ] `cargo test --all-features --locked`
- [ ] `cargo test --locked --test performance -- --nocapture`
- [ ] `python3 -B -m unittest discover -s tests/performance -p '*_test.py'`
- [ ] `cargo build --release --features native-toolchain --locked`

## CI topology 与 exact SHA

- [ ] quality + native-integration + 六 native-host + 两 performance 恰好十 job。
- [ ] 六 host 均运行非零 profile runtime、publication、artifact-path、void-call、
  dynamic/executable selector，无 required skip。
- [ ] 两 stable Linux host 均按序通过 schema7/8/9 与 Contract 1 collector/checker，
  上传闭合证据。
- [ ] checkout、candidate compiler、两个 report、artifact 与 workflow head SHA
  完全相等；十 job 对同一最终 SHA 成功。

## 交付

- [ ] `99-final-acceptance.md` 全部静态与本地项由 final SHA 真实结果支持。
- [ ] 动态日志只在 ignored acceptance 目录/CI artifact；source commit 无 profile、
  report、decision、cache、run id 或 secret。
- [ ] feature branch 已提交并推送，worktree clean；main 未合并，tag/Release 未创建。
- [ ] v0.13 accepted-base 状态单独记录，未用移动引用或旧 run 代签。

## 完成证据

本地命令、test count、toolchain/host identity 写入
`target/acceptance/v0.14/final/`；远程 run/job/artifact identity 保存在 CI artifact
和该 ignored 目录。
