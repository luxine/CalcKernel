# 阶段 19 任务：十作业 CI、全量回归与最终交付

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

在不改变十个 required job 拓扑的前提下，把阶段 12–18 接入 quality、
native-integration、六 host 和两 stable performance job；在同一最终 SHA 回归
阶段 01–11 与所有新验收，提交并推送 feature branch，低频检查 exact-SHA CI，
不合并 main。

## 仓库落点与接口

- 修改 `.github/workflows/ci.yml`：
  - quality 运行 Rust performance contract、全部 Python checker mutation 和 CI
    contract；
  - native-integration 运行 predicated optimizer/attestation/runner、profile
    runtime、void-call、真实 dynamic/executable 和 artifact audit；
  - 六 host 的 required native suite显式包含 profile durable reopen、publication
    success/fault、host paths、void call、dynamic/executable；
  - 两 performance job 在 schema 9 通过后执行 Contract 1 collector/checker，
    并上传 report、完整 evidence tree和日志。
- 扩展 `tests/contracts/ci.rs`，断言十 job、六 matrix row、两 tier、exact SHA、
  非零 selector、顺序、artifact path、无 skip/continue-on-error/threshold bypass。
- 更新 `specs/0.14/implementation/99-final-acceptance.md` 的动态执行记录只写
  ignored `target/acceptance/v0.14/final/`。

## TDD 与执行顺序

1. 添加 CI RED `ci_v014_native_fulfillment_should_run_on_all_six_hosts` 与
   `ci_v014_predicated_update_should_gate_both_performance_hosts`；确认 workflow
   缺少新 selectors/collector/checker/upload 时失败。
2. 修改 workflow，保持 job cardinality=10；所有新增命令进入已有 required
   step，performance 顺序严格 schema7→schema8→schema9→Contract1。
3. 运行 `cargo fmt --check`、clippy、rustdoc、feature-disabled、all-features、
   contracts、optimizer、tune、CLI、Native、performance 和 Python unit tests；
   每个 selector先用 `--list` 或输出 test count证明非零。
4. 使用 `/tmp/ckc-llvm-v013-prefix`（若 manifest/22.1.8 身份通过）执行本机
   Native 全集、真实 Floyd collect/checker；不符合 stable CPU tier 时只记录
   capability并让远程真实性能门禁负责，不能本地伪签。
5. 执行 `99-final-acceptance.md`；任何代码/fixture/checker/spec 变化后重跑受
   影响阶段与最终静态门禁。确认 worktree 只含预期更改。
6. 创建清晰 final implementation commit，push
   `design/v0.14-offline-autotuning`，用 workflow_dispatch 指向 exact branch/SHA；
   记录 run id 和 head SHA。
7. 远程运行期间至少间隔 15 分钟查询一次，不在前台原地等待。暂态基础设施
   失败可重跑同 SHA；产品、checker、fixture 或 plan 变化必须提交新 SHA 并重跑
   全部受影响 job。
8. 十 job 对同一 SHA 全部成功后验证两个 Contract 1 report 的 candidate SHA、
   compiler、decision、artifact、receipt 和 evidence inventory；保持 worktree
   clean，不 merge main，不创建 tag/Release，等待用户审查。

## 最终本地命令

```sh
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --locked
cargo test --locked
cargo test --all-features --locked
cargo test --locked --test performance -- --nocapture
python3 -B -m unittest discover -s tests/performance -p '*_test.py'
cargo build --release --features native-toolchain --locked
```

## 边界

- 不增加第十一个 job，不减少 matrix row，不把 required test/performance 改为
  diagnostic、optional、continue-on-error 或空 selector。
- 不自动合并 main，不创建或移动 tag/Release。
- v0.13 accepted-base 必须精确为 `4cbaa0fb970a5ee2112d5d4f54d1a6e0186f875a`；逐差异
  审计与等价集成是 release gate，不能由旧 run 或移动引用代签。
