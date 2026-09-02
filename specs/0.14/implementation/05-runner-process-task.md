# 阶段 05 任务：runner protocol、containment 与 calibration

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

实现跨平台、无 shell、空环境的 harness 调用器，冻结 CKTUNE/1 stdout、外部单调计时、输出上限、
POSIX process group/Windows Job Object cooperative containment、完整 timeout 与 baseline calibration。

## 仓库落点与接口

- 新建 `src/tune/runner/{mod.rs,protocol.rs,timer.rs,process_unix.rs,process_windows.rs}`。
- `TuneRunner::invoke(&CapturedWorkload, &NonPublishableTuneTrial, &Invocation)` 返回
  `InvocationResult { elapsed_ns, completed, digest }` 或闭合 `RunnerFailure/CanonicalCandidateTimeout`。
- `calibrate_cases` 以 case-id 顺序执行 doubling 1..32、>=50 ms 接受、同 iterations confirmation、
  >250 ms overshoot，并将所有 baseline correctness 绑定 manifest expected digest。
- 新增 `tests/tune/{runner.rs,calibration.rs}` 和 retained probe binaries；平台行为扩展
  `tests/native/run.rs`，Windows argv fixture 覆盖 empty/space/quote/trailing slash/non-ASCII。

## TDD 顺序

1. 写 protocol RED：stdout 必须是唯一 `CKTUNE/1 ...\n`，exact echo/completed/digest/status；extra、>4 KiB、
   invalid UTF-8、stderr >1 MiB、crash/signal/nonzero/错误 digest 均分类为 session abort。
2. 写 process RED：argv/env 直接传递、cwd=`CK_TUNE_TEMP`、每次 fresh inputs；Unix 在 runner 执行前建新
   group，Windows suspended→non-breakaway Job+KILL_ON_JOB_CLOSE→resume，建立失败不得运行用户代码。
3. 写 timeout RED：只使用完整 configured timeout，超时/overflow 先 cooperative terminate，250 ms 后
   force terminate，并在 2,000 ms 内 reaping/empty；failure abort，合法 complete candidate timeout 单独分类。
4. 写 Windows UCRT inverse RED：总是 quote argv0+args，反斜线/quote/closing quote 精确恢复，使用独立
   `lpApplicationName`；Unix 保留 exact accepted UTF-8 bytes。
5. 写 calibration RED：每个 search+validation case 1 开始 checked doubling、最多 32、50 ms threshold、
   confirmation、overshoot；baseline timeout/correctness/overflow abort，iterations 不在 validation 重算。
6. 写 wall admission RED：调用前剩余 budget 必须 >= timeout+2250 ms，不能缩短 deadline；未启动和真实
   complete candidate timeout 是不同状态。
7. 运行 `cargo test --test tune runner_ -- --nocapture`、`calibration_`、`cargo test --test native run_` 和
   `cargo test --all-features --locked`，记录平台 probe 与 RED/GREEN。

## 实现边界

- POSIX setsid/double-fork escape 是规范明确排除的 hostile behavior；测试只证明 cooperative contract 和拒绝承诺。
- correctness mismatch 永远不是“慢候选”或 baseline fallback。
- raw stdout/stderr 不进入 decision；stderr 仅作为 bounded local diagnostic。

